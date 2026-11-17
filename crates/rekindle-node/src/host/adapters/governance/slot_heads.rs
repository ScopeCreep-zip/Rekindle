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

use super::HostGovernance;

/// Where a departing member writes channel posts: its segment and its slot
/// in that segment's channel records, and whether it is us.
struct DepartedSlot {
    segment: u32,
    slot_index: u32,
    own: bool,
}

impl HostGovernance {
    /// The heads of `target_hex`'s slot in each channel record of its
    /// segment, as our local copy holds them (our own slot, for a leave, is
    /// one we wrote every value of). Never a network read: the removal is
    /// authored at the state we hold, and a round trip before it lets ops
    /// that arrive meanwhile into its causal past, so a kick concurrent
    /// with the remover's own removal would no longer be void (netsim
    /// `a_removal_concurrent_with_the_removers_own_is_void_everywhere`).
    ///
    /// A record where we hold no value of the slot gets no head: the member
    /// wrote none there, or none reached us. Our own leave signs every head
    /// or fails: a record we cannot read would take our posts in it out of
    /// every reader's history for good, so the leave is refused instead. A
    /// remover signs what it holds and warns of what it cannot read, so a
    /// lagging roster or record never blocks a removal.
    pub(super) async fn departed_slot_heads_impl(
        &self,
        community_id: &str,
        target_hex: &str,
    ) -> Result<Vec<SlotHead>, GovernanceRuntimeError> {
        let membership = (self.community_membership_impl(community_id))
            .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_owned()))?;
        let Some(departed) = self.departed_slot(community_id, &membership, target_hex)? else {
            return Ok(Vec::new());
        };
        let slot = u32::from(OWNER_SUBKEY_COUNT) + departed.slot_index;
        let pool = self.transport()?.require_records().map_err(adapter)?;
        let mut heads = Vec::new();
        for record_key in self.segment_channel_records(community_id, membership, departed.segment) {
            let lease = match pool.acquire(&record_key, None, OpenIntent::Check).await {
                Ok(lease) => lease,
                Err(e) if departed.own => return Err(adapter(e)),
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
            // A local inspect, not a get: a get without refresh still asks
            // the network when we hold no value (veilid-core 0.5.8
            // `storage_manager/get_value.rs:155-165` returns early only on a
            // local hit), which held the kick ~11 s in the netsim. Only a
            // slot the inspect found locally is then read, a local hit.
            let read = local_slot_value(&*pool, lease, slot).await;
            pool.release(lease).await;
            match read {
                Ok(Some(value)) => heads.push(SlotHead {
                    digest: rekindle_secrets::slot_head::slot_head_digest(
                        &record_key,
                        slot,
                        value.seq,
                        &value.data,
                    ),
                    record_key,
                    seq: value.seq,
                }),
                Ok(None) => {}
                Err(e) if departed.own => return Err(adapter(e)),
                Err(e) => {
                    tracing::warn!(
                        community_id,
                        record_key,
                        error = %e,
                        "slot head not read for a removal; the member's posts in the record are kept by no reader"
                    );
                }
            }
        }
        Ok(heads)
    }

    /// The departing member's segment and slot: ours from our membership,
    /// as our own `channel_write_context` writes it; a member's from the
    /// roster. A member the roster has not placed yet gets none, with a
    /// warning: its posts are then kept by no reader.
    fn departed_slot(
        &self,
        community_id: &str,
        membership: &CommunityMembership,
        target_hex: &str,
    ) -> Result<Option<DepartedSlot>, GovernanceRuntimeError> {
        let own = (membership.my_pseudonym_hex.as_deref())
            .is_some_and(|me| me.eq_ignore_ascii_case(target_hex));
        if own {
            let slot_index = membership
                .my_subkey_index
                .ok_or_else(|| GovernanceRuntimeError::SlotIndexMissing(community_id.to_owned()))?;
            return Ok(Some(DepartedSlot {
                segment: membership.my_segment_index.unwrap_or(0),
                slot_index,
                own,
            }));
        }
        let placed = (self.ctx.communities.members(community_id).into_iter())
            .find(|(pseudonym, _)| pseudonym.eq_ignore_ascii_case(target_hex))
            .map(|(_, member)| DepartedSlot {
                segment: member.segment_index,
                slot_index: member.subkey_index,
                own,
            });
        if placed.is_none() {
            tracing::warn!(
                community_id,
                target = target_hex,
                "removed member not on the roster; the removal records no slot heads, so its posts are kept by no reader"
            );
        }
        Ok(placed)
    }

    /// The channel records `segment`'s members write: segment 0's from
    /// `ChannelCreated`, a later one's from `ChannelSegmentLinked`, as
    /// `channel_write_context` picks them.
    fn segment_channel_records(
        &self,
        community_id: &str,
        membership: CommunityMembership,
        segment: u32,
    ) -> Vec<String> {
        let mut record_keys: Vec<String> = if segment == 0 {
            membership.channel_log_keys.into_values().collect()
        } else {
            (self.governance_state_impl(community_id).into_iter())
                .flat_map(|gov| gov.channel_segment_records.into_iter())
                .filter(|((_, linked), _)| *linked == segment)
                .map(|(_, record)| record.record_key)
                .collect()
        };
        record_keys.sort_unstable();
        record_keys.dedup();
        record_keys
    }
}

/// Our local copy of `slot`'s value, never read from the network: the
/// head commits to its seq and its decrypted page (plan S9), so a reader
/// keeps the member's posts only from exactly that value. `None` when we
/// hold no value of the slot.
async fn local_slot_value(
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
