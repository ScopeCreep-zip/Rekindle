//! The slot heads a departure records (A/B finding R3-8): for each channel
//! record of the departing member's segment, the seq of its slot value as
//! we hold it when we remove it, or of our own when we leave, and the
//! digest of that value's page (plan S9). The
//! `MemberRemoved` or our `MemberLeft` carries them, signed, to every
//! reader, so every reader keeps the same of the member's posts from
//! history: those read from exactly its head value, and none from a record
//! with no head. The digest closes a leaver's reserved seq gap: a head
//! signed above the published seq names a page that a later write into the
//! gap (veilid-core 0.5.8 `set_value` `min_seqnum`) never matches. A reader computing the heads itself at merge time
//! would hold a different value than its peers and keep different posts.
//!
//! Our own slot is found from our membership, never from the roster: the
//! registry poll skips our own row, so the roster never holds us (A/B
//! 2026-11-17: a leave read its slot from the roster, found none, and
//! signed no heads, so a member joining after it kept none of our posts).

use rekindle_governance_runtime::deps::CommunityMembership;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_records::lease::LeaseId;
use rekindle_records::port::{OpenIntent, PortError, RecordPort, RecordValue, ReportScope};
use rekindle_types::dht_layout::channel::OWNER_SUBKEY_COUNT;
use rekindle_types::governance::SlotHead;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// The heads of `target_hex`'s slot values in the channel records, as
    /// our local copy holds them. Never a network read: the removal is
    /// authored at the state we hold, and a round trip before it lets ops
    /// that arrive meanwhile into its causal past, so a kick concurrent
    /// with the remover's own removal would no longer be void (netsim
    /// `a_removal_concurrent_with_the_removers_own_is_void_everywhere`).
    ///
    /// Our own leave heads our slot in each record of our segment, one we
    /// wrote every value of, and signs every head or fails: a record we
    /// cannot read would take our posts in it out of every reader's history
    /// for good, so the leave is refused instead. A remover heads every
    /// slot value it holds whose page the target signed (`decode_own_page`,
    /// the author's pseudonym signature): which slot a member writes is
    /// what its pages show, never the presence roster, which the registry
    /// poll fills a poll interval after a (re-)join (A/B F8-1: a kick 25 s
    /// after a re-join found the member on no roster and signed no heads,
    /// so every reader backfilling later dropped all its posts). A record
    /// where we hold no page of the member gets no head: the member wrote
    /// none there, or none reached us. A remover warns of what it cannot
    /// read and signs the rest, so a lagging record never blocks a removal.
    pub(super) async fn departed_slot_heads_impl(
        &self,
        community_id: &str,
        target_hex: &str,
    ) -> Result<Vec<SlotHead>, GovernanceRuntimeError> {
        let membership = (self.community_membership_impl(community_id))
            .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_owned()))?;
        let own = (membership.my_pseudonym_hex.as_deref())
            .is_some_and(|me| me.eq_ignore_ascii_case(target_hex));
        if own {
            self.own_slot_heads(community_id, membership).await
        } else {
            let target = (hex::decode(target_hex).ok())
                .and_then(|bytes| bytes.try_into().ok())
                .map(PseudonymKey)
                .ok_or_else(|| adapter(format!("not a pseudonym: {target_hex}")))?;
            self.removed_slot_heads(community_id, membership, &target)
                .await
        }
    }

    /// Our own slot's head in each channel record of our segment, our slot
    /// from our membership, as our own `channel_write_context` writes it.
    async fn own_slot_heads(
        &self,
        community_id: &str,
        membership: CommunityMembership,
    ) -> Result<Vec<SlotHead>, GovernanceRuntimeError> {
        let slot_index = membership
            .my_subkey_index
            .ok_or_else(|| GovernanceRuntimeError::SlotIndexMissing(community_id.to_owned()))?;
        let segment = membership.my_segment_index.unwrap_or(0);
        let slot = u32::from(OWNER_SUBKEY_COUNT) + slot_index;
        let pool = self.transport()?.require_records().map_err(adapter)?;
        let mut heads = Vec::new();
        for record_key in self.channel_records(community_id, membership, Some(segment)) {
            let lease =
                (pool.acquire(&record_key, None, OpenIntent::Check).await).map_err(adapter)?;
            // A local inspect, not a get: a get without refresh still asks
            // the network when we hold no value (veilid-core 0.5.8
            // `storage_manager/get_value.rs:155-165` returns early only on a
            // local hit), which held the kick ~11 s in the netsim. Only a
            // slot the inspect found locally is then read, a local hit.
            let read = local_slot_value(&*pool, lease, slot).await;
            pool.release(lease).await;
            if let Some(value) = read.map_err(adapter)? {
                heads.push(slot_head(record_key, slot, &value));
            }
        }
        Ok(heads)
    }

    /// The head of every slot value we hold, in any channel record of any
    /// segment, whose page `target` signed.
    async fn removed_slot_heads(
        &self,
        community_id: &str,
        membership: CommunityMembership,
        target: &PseudonymKey,
    ) -> Result<Vec<SlotHead>, GovernanceRuntimeError> {
        let pool = self.transport()?.require_records().map_err(adapter)?;
        let mut heads = Vec::new();
        for record_key in self.channel_records(community_id, membership, None) {
            let lease = match pool.acquire(&record_key, None, OpenIntent::Check).await {
                Ok(lease) => lease,
                Err(e) => {
                    tracing::warn!(
                        community_id,
                        record_key,
                        error = %e,
                        "channel record not opened for a removal head; the member's posts in it are kept by no reader"
                    );
                    continue;
                }
            };
            let read = local_pages_of(&*pool, lease, target).await;
            pool.release(lease).await;
            match read {
                Ok(pages) => heads.extend(
                    (pages.into_iter())
                        .map(|(slot, value)| slot_head(record_key.clone(), slot, &value)),
                ),
                Err(e) => {
                    tracing::warn!(
                        community_id,
                        record_key,
                        error = %e,
                        "slot heads not read for a removal; the member's posts in the record are kept by no reader"
                    );
                }
            }
        }
        if heads.is_empty() {
            tracing::info!(
                community_id,
                target = %hex::encode(target.0),
                "removed member wrote no page we hold; the removal records no slot heads"
            );
        }
        Ok(heads)
    }

    /// The channel records `segment`'s members write (every segment's when
    /// `None`): segment 0's from `ChannelCreated`, a later one's from
    /// `ChannelSegmentLinked`, as `channel_write_context` picks them.
    fn channel_records(
        &self,
        community_id: &str,
        membership: CommunityMembership,
        segment: Option<u32>,
    ) -> Vec<String> {
        let mut record_keys: Vec<String> = if segment.is_none_or(|s| s == 0) {
            membership.channel_log_keys.into_values().collect()
        } else {
            Vec::new()
        };
        if segment != Some(0) {
            record_keys.extend(
                (self.governance_state_impl(community_id).into_iter())
                    .flat_map(|gov| gov.channel_segment_records.into_iter())
                    .filter(|((_, linked), _)| segment.is_none_or(|s| *linked == s))
                    .map(|(_, record)| record.record_key),
            );
        }
        record_keys.sort_unstable();
        record_keys.dedup();
        record_keys
    }
}

/// The departure head of `slot`'s `value` in `record_key`: its seq and the
/// digest of its page (plan S9).
fn slot_head(record_key: String, slot: u32, value: &RecordValue) -> SlotHead {
    SlotHead {
        digest: rekindle_secrets::slot_head::slot_head_digest(
            &record_key,
            slot,
            value.seq,
            &value.data,
        ),
        record_key,
        seq: value.seq,
    }
}

/// Every writer slot's value we hold locally whose page `author` signed,
/// never read from the network: one local inspect of every subkey, then a
/// local hit for each held writer slot.
async fn local_pages_of(
    pool: &dyn RecordPort,
    lease: LeaseId,
    author: &PseudonymKey,
) -> Result<Vec<(u32, RecordValue)>, PortError> {
    let report = pool.inspect(lease, None, ReportScope::Local).await?;
    let held = (report.subkeys.iter().zip(&report.local_seqs))
        .filter(|(subkey, seq)| **subkey >= u32::from(OWNER_SUBKEY_COUNT) && seq.is_some())
        .map(|(subkey, _)| *subkey);
    let mut pages = Vec::new();
    for slot in held.collect::<Vec<_>>() {
        if let Some(value) = pool.get(lease, slot, false).await? {
            if rekindle_codec::community::channel_record::decode_own_page(&value.data, author)
                .is_some()
            {
                pages.push((slot, value));
            }
        }
    }
    Ok(pages)
}

/// Our local copy of `slot`'s value, never read from the network: the
/// head commits to its seq and its decrypted page (plan S9), so a reader
/// keeps the member's posts only from exactly that value. `None` when we
/// hold no value of the slot.
pub(super) async fn local_slot_value(
    pool: &dyn RecordPort,
    lease: LeaseId,
    slot: u32,
) -> Result<Option<RecordValue>, PortError> {
    let report = pool
        .inspect(lease, Some(&[slot]), ReportScope::Local)
        .await?;
    let held = (report.subkeys.iter().zip(&report.local_seqs))
        .any(|(subkey, seq)| *subkey == slot && seq.is_some());
    if !held {
        return Ok(None);
    }
    pool.get(lease, slot, false).await
}

fn adapter(e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(e.to_string())
}
