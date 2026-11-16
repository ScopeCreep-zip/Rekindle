//! The slot heads a departure records (A/B finding R3-8): for each channel
//! record the history reads, the seq of the departing member's slot value
//! as we hold it when we remove it, or of our own when we leave. The
//! `MemberRemoved` or our `MemberLeft` carries them, signed, to every
//! reader, so every reader keeps the same of the member's posts from
//! history: those read from a value at or below its head. A reader
//! computing the heads itself at merge time would hold a different value
//! than its peers and keep different posts.

use rekindle_records::port::{OpenIntent, ReportScope};
use rekindle_types::dht_layout::channel::OWNER_SUBKEY_COUNT;
use rekindle_types::governance::SlotHead;

use super::HostGovernance;

impl HostGovernance {
    /// The heads of `target_hex`'s slot in each of `community_id`'s channel
    /// records, as our local copy holds them (our own slot, for a leave, is
    /// one we wrote every value of). Never a network read: the
    /// removal is authored at the state we hold, and a round trip before it
    /// lets ops that arrive meanwhile into its causal past, so a kick
    /// concurrent with the remover's own removal would no longer be void
    /// (netsim `a_removal_concurrent_with_the_removers_own_is_void_everywhere`).
    /// A record we cannot open, or where we hold no value of the slot, gets
    /// no head, and a removed member's post is then kept from it by no
    /// reader.
    pub(super) async fn departed_slot_heads_impl(
        &self,
        community_id: &str,
        target_hex: &str,
    ) -> Vec<SlotHead> {
        let members = self.ctx.communities.members(community_id);
        let Some(slot) = (members.iter())
            .find(|(pseudonym, _)| pseudonym.eq_ignore_ascii_case(target_hex))
            .map(|(_, member)| u32::from(OWNER_SUBKEY_COUNT) + member.subkey_index)
        else {
            return Vec::new();
        };
        let Some(membership) = self.community_membership_impl(community_id) else {
            return Vec::new();
        };
        let Ok(pool) = self.transport().and_then(|t| {
            t.require_records().map_err(|e| {
                rekindle_governance_runtime::GovernanceRuntimeError::Adapter(e.to_string())
            })
        }) else {
            return Vec::new();
        };
        let mut record_keys: Vec<String> = membership.channel_log_keys.into_values().collect();
        record_keys.sort_unstable();
        record_keys.dedup();
        let mut heads = Vec::new();
        for record_key in record_keys {
            let Ok(lease) = pool.acquire(&record_key, None, OpenIntent::Check).await else {
                tracing::debug!(
                    community_id,
                    record_key,
                    "channel record not opened for a head"
                );
                continue;
            };
            // A local inspect, not a get: a get without refresh still asks
            // the network when we hold no value (veilid-core 0.5.8
            // `storage_manager/get_value.rs:155-165` returns early only on a
            // local hit), which held the kick ~11 s in the netsim.
            let read = (pool.inspect(lease, Some(&[slot]), ReportScope::Local)).await;
            pool.release(lease).await;
            match read {
                Ok(report) => heads.extend(
                    (report.subkeys.iter().zip(&report.local_seqs))
                        .filter(|(subkey, _)| **subkey == slot)
                        .filter_map(|(_, seq)| *seq)
                        .map(|seq| SlotHead {
                            record_key: record_key.clone(),
                            seq,
                        }),
                ),
                Err(e) => {
                    tracing::debug!(community_id, record_key, error = %e, "slot head not read");
                }
            }
        }
        heads
    }
}
