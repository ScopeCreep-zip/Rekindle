//! Our posts written past our own removal (plan E3.2 S9, finding F9-3).
//! A post we wrote after the remover took our slot heads lands, but every
//! reader drops it (`NotMember`: a departed author's post is kept only from
//! exactly the slot value its departures recorded,
//! `rekindle_channel::ingest::predates_departure`). So when our removal is
//! merged, each of our posts in a slot value past that head, and each post
//! whose write had not landed yet, reads `failed` with
//! [`REMOVED_NOT_DELIVERED`] instead of `sent`. Discord marks a message a
//! removed user tried to send as not delivered; Signal shows "not
//! delivered" when you were removed from a group before it went out.
//!
//! The head is read from the merged state, `GovernanceState::departed_heads`
//! for our pseudonym, the very set every reader judges by. A head names a
//! slot value by its seq and the digest of its page
//! (`rekindle_secrets::slot_head::slot_head_digest`), not by which of our
//! slots it is, so it is matched against our own pages: our current slot's
//! value as we hold it locally (we wrote every value of it), else each
//! earlier page of it, which is that page's entries up to the post past it
//! (an append adds one entry at the end, `channel_record/write.rs`), signed
//! again (Ed25519 signatures are deterministic, the page JSON is the same
//! encoder's). The posts after the matched page are the ones no reader
//! keeps. A slot no head of ours reproduces is left as it is: the removal
//! recorded nothing to compare our posts with.

use std::collections::BTreeSet;

use rekindle_codec::community::channel_record::{
    decode_own_page, encode_page_entries, ChannelRecordEntry,
};
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::CommunityMembership;
use rekindle_records::port::OpenIntent;
use rekindle_types::dht_layout::channel::OWNER_SUBKEY_COUNT;
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::id::{ChannelId, PseudonymKey};

use super::HostGovernance;
use crate::host::adapters::channel::not_delivered::{fail_not_delivered, OwnPost};
use crate::host::adapters::channel::HostChannel;

/// What a post written past our removal reads (plan E3.2 S9, F9-3).
pub(crate) const REMOVED_NOT_DELIVERED: &str = "Not delivered: you were removed from the community";

impl HostGovernance {
    /// Fail every post of ours in `community_id` that no reader keeps since
    /// our removal: those in our slot past the head the removal recorded,
    /// and those whose write had not landed yet.
    pub(super) async fn fail_posts_past_removal(&self, community_id: &str) {
        let Some(membership) = self.community_membership_impl(community_id) else {
            return;
        };
        let (Some(me), Some(slot_index)) = (
            (membership.my_pseudonym_hex.as_deref()).and_then(pseudonym_of),
            membership.my_subkey_index,
        ) else {
            return;
        };
        let (Some(gov), Some(secret)) = (
            self.governance_state_impl(community_id),
            self.identity_secret_impl(),
        ) else {
            return;
        };
        let signing_key =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
        let slot = u32::from(OWNER_SUBKEY_COUNT) + slot_index;
        let heads = gov.departed_heads.get(&me).cloned().unwrap_or_default();
        let mut posts = Vec::new();
        for (channel, record_key) in own_channel_records(&membership, &gov) {
            if let Some(outbox) = self.ctx.outbox.channel_posts() {
                match outbox
                    .fail_slot(&record_key, slot, REMOVED_NOT_DELIVERED)
                    .await
                {
                    Ok(unlanded) => posts.extend(unlanded.into_iter().map(|post| OwnPost {
                        channel: post.channel,
                        message: post.message,
                    })),
                    Err(error) => {
                        tracing::warn!(community = %community_id, %error, "unlanded posts not failed at our removal");
                    }
                }
            }
            let Some(page) = self.own_page(&record_key, slot, &me).await else {
                continue;
            };
            let no_heads = BTreeSet::new();
            let held = heads.get(&record_key).unwrap_or(&no_heads);
            let encode = |prefix: &[ChannelRecordEntry]| {
                encode_page_entries(me.clone(), &signing_key, prefix.to_vec()).ok()
            };
            let Some(kept) = first_past_head(&record_key, slot, &page, held, encode) else {
                tracing::info!(community = %community_id, record = %record_key, "no head of our removal reproduces our page; its posts are left as they are");
                continue;
            };
            posts.extend(
                (page.entries[kept..].iter())
                    .filter_map(entry_message_id)
                    .map(|message| OwnPost {
                        channel: channel.clone(),
                        message,
                    }),
            );
        }
        posts.extend(
            self.outbox_posts_past_removal(community_id, &me, &gov)
                .await,
        );
        let failed = fail_not_delivered(self, community_id, &posts, REMOVED_NOT_DELIVERED).await;
        if failed > 0 {
            tracing::info!(community = %community_id, failed, "posts written past our removal marked not delivered");
        }
    }

    /// Our channel outbox's posts no reader keeps since our removal (plan
    /// E3.3): every post still unsettled on a channel's stream, and every
    /// entry of our outbox above the highest head of it our departures
    /// recorded that our chain reproduces. With no such head the removal
    /// recorded nothing to compare our entries with, and they are left as
    /// they are (as a slot no head reproduces).
    async fn outbox_posts_past_removal(
        &self,
        community_id: &str,
        me: &PseudonymKey,
        gov: &GovernanceState,
    ) -> Vec<OwnPost> {
        let mut posts = Vec::new();
        if let Some(outbox) = self.ctx.outbox.channel_posts() {
            for channel in gov.channels.keys().map(|id| id.to_hex()) {
                match (outbox.fail_channel(community_id, &channel, REMOVED_NOT_DELIVERED)).await {
                    Ok(unlanded) => posts.extend(unlanded.into_iter().map(|post| OwnPost {
                        channel: post.channel,
                        message: post.message,
                    })),
                    Err(error) => {
                        tracing::warn!(community = %community_id, %error, "unsent posts not failed at our removal");
                    }
                }
            }
        }
        // Our record from the merged state: the removal's teardown has
        // already dropped our membership's.
        let Some(record) = gov.op_log_records.get(me).cloned() else {
            return posts;
        };
        let Ok(pool) = (self.transport().ok().map(|t| t.require_records())).transpose() else {
            return posts;
        };
        let Some(pool) = pool else {
            return posts;
        };
        let read = rekindle_records::chain::read_log_after(&*pool, &record, OUTBOX, 0).await;
        let Ok(chain) = read.inspect_err(|e| tracing::warn!(community = %community_id, error = %e, "our outbox not read at our removal")) else {
            return posts;
        };
        let heads = (gov.departed_heads.get(me))
            .and_then(|heads| heads.get(&record).cloned())
            .unwrap_or_default();
        let kept = heads.iter().rev().find_map(|(seq, digest)| {
            let at = chain.entries.iter().find(|e| e.seq == u64::from(*seq))?;
            (super::slot_heads::outbox_head_digest(&record, *seq, &at.ciphertext) == *digest)
                .then_some(u64::from(*seq))
        });
        let Some(kept) = kept else {
            return posts;
        };
        let at = |seq| rekindle_codec::member_record::ChainPosition {
            anchor: community_id,
            chain_seq: seq,
        };
        for entry in chain.entries.iter().filter(|e| e.seq > kept) {
            let Ok(decoded) = rekindle_codec::member_record::decode_outbox_entry(
                &entry.ciphertext,
                at(entry.seq),
                &me.0,
            ) else {
                continue;
            };
            let Some(message) = entry_message_id(&decoded.entry) else {
                continue;
            };
            let channel = HostChannel::of_governance(self).channel_of_tag(
                community_id,
                decoded.generation,
                &decoded.channel_tag,
            );
            posts.extend(channel.map(|channel| OwnPost { channel, message }));
        }
        posts
    }

    /// Our current slot's value in `record_key` as we hold it locally, and
    /// its entries, when we hold one we signed. Never a network read.
    async fn own_page(&self, record_key: &str, slot: u32, me: &PseudonymKey) -> Option<OwnPage> {
        let pool = (self.transport().ok())?.require_records().ok()?;
        let lease = (pool.acquire(record_key, None, OpenIntent::Check).await)
            .inspect_err(|e| tracing::warn!(record = %record_key, error = %e, "our page not opened at our removal"))
            .ok()?;
        let read = super::slot_heads::local_slot_value(&*pool, lease, slot).await;
        pool.release(lease).await;
        let value = read
            .inspect_err(|e| tracing::warn!(record = %record_key, error = %e, "our page not read at our removal"))
            .ok()??;
        let entries = decode_own_page(&value.data, me)?;
        Some(OwnPage {
            seq: value.seq,
            data: value.data,
            entries,
        })
    }
}

/// Our slot's value: its seq, its decrypted page and the page's entries.
pub(super) struct OwnPage {
    pub seq: u32,
    pub data: Vec<u8>,
    pub entries: Vec<ChannelRecordEntry>,
}

/// How many of `page`'s entries a reader keeps: all of them when `page`
/// itself is a recorded head, else the entries of the latest earlier page
/// of the slot that is one (`encode` signs a prefix of entries as that
/// page was written). `None` when no head in `heads` is a page of this
/// slot we can reproduce.
pub(super) fn first_past_head(
    record_key: &str,
    slot: u32,
    page: &OwnPage,
    heads: &BTreeSet<(u32, [u8; 32])>,
    encode: impl Fn(&[ChannelRecordEntry]) -> Option<Vec<u8>>,
) -> Option<usize> {
    let digest = |seq: u32, bytes: &[u8]| {
        rekindle_secrets::slot_head::slot_head_digest(record_key, slot, seq, bytes)
    };
    if heads.contains(&(page.seq, digest(page.seq, &page.data))) {
        return Some(page.entries.len());
    }
    (0..page.entries.len()).rev().find(|kept| {
        encode(&page.entries[..*kept]).is_some_and(|bytes| {
            (heads.iter()).any(|(seq, head)| *seq < page.seq && *head == digest(*seq, &bytes))
        })
    })
}

/// The message id a post entry carries (a message or a forward).
fn entry_message_id(entry: &ChannelRecordEntry) -> Option<String> {
    match entry {
        ChannelRecordEntry::Message(message) => message.message_id.clone(),
        ChannelRecordEntry::Forward(forward) => forward.message_id.clone(),
        _ => None,
    }
}

/// The channel records our slot writes, each with its channel: our
/// segment's, as `channel_write_context` picks them (segment 0's from
/// `ChannelCreated`, a later one's from `ChannelSegmentLinked`).
fn own_channel_records(
    membership: &CommunityMembership,
    gov: &GovernanceState,
) -> Vec<(String, String)> {
    let segment = membership.my_segment_index.unwrap_or(0);
    if segment == 0 {
        return (membership.channel_log_keys.iter())
            .map(|(channel, record)| (channel.clone(), record.clone()))
            .collect();
    }
    (gov.channel_segment_records.iter())
        .filter(|((_, linked), _)| *linked == segment)
        .map(|((channel, _), record): (&(ChannelId, u32), _)| {
            (hex::encode(channel.0), record.record_key.clone())
        })
        .collect()
}

fn pseudonym_of(hex_key: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_key).ok()?;
    Some(PseudonymKey(bytes.try_into().ok()?))
}

#[cfg(test)]
#[path = "removal_posts_tests.rs"]
mod tests;
