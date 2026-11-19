//! Our posts written past our own removal (plan E3.2 S9, finding F9-3).
//! A post we wrote after the remover took our outbox head lands, but every
//! reader drops it (`NotMember`: a departed author's post is kept only at or
//! below the head its departures recorded,
//! `rekindle_channel::ingest::predates_departure`). So when our removal is
//! merged, each of our outbox entries past that head, and each post whose
//! write had not landed yet, reads `failed` with [`REMOVED_NOT_DELIVERED`]
//! instead of `sent`. Discord marks a message a removed user tried to send
//! as not delivered; Signal shows "not delivered" when you were removed
//! from a group before it went out.
//!
//! The head is read from the merged state, `GovernanceState::departed_heads`
//! for our pseudonym, the very set every reader judges by. A channel has no
//! record (plan E3.3), so our channel outbox (`M`/4) is the one place our
//! posts are.

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_governance::state::GovernanceState;
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;
use crate::host::adapters::channel::not_delivered::{fail_not_delivered, OwnPost};
use crate::host::adapters::channel::HostChannel;

/// What a post written past our removal reads (plan E3.2 S9, F9-3).
pub(crate) const REMOVED_NOT_DELIVERED: &str = "Not delivered: you were removed from the community";

impl HostGovernance {
    /// Fail every post of ours in `community_id` that no reader keeps since
    /// our removal: our outbox entries past the head the removal recorded,
    /// and those whose write had not landed yet.
    pub(super) async fn fail_posts_past_removal(&self, community_id: &str) {
        let Some(membership) = self.community_membership_impl(community_id) else {
            return;
        };
        let Some(me) = (membership.my_pseudonym_hex.as_deref()).and_then(pseudonym_of) else {
            return;
        };
        let Some(gov) = self.governance_state_impl(community_id) else {
            return;
        };
        let posts = self
            .outbox_posts_past_removal(community_id, &me, &gov)
            .await;
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
}

/// The message id a post entry carries (a message or a forward).
fn entry_message_id(entry: &ChannelRecordEntry) -> Option<String> {
    match entry {
        ChannelRecordEntry::Message(message) => message.message_id.clone(),
        ChannelRecordEntry::Forward(forward) => forward.message_id.clone(),
        _ => None,
    }
}

fn pseudonym_of(hex_key: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_key).ok()?;
    Some(PseudonymKey(bytes.try_into().ok()?))
}

