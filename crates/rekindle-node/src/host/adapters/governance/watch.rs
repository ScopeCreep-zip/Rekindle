//! The governance arm of the Host's watch path (plan E3.2 S7, D4), on both
//! modes: a ValueChange on the frontier (`M`/3) of a member record our watch
//! ring holds, at a frontier seq we have not read, reads that frontier,
//! stores the heads we lack, and rebuilds that community alone; then
//! borrows every channel record the new state names that we do not hold.
//! This (with a `HeadAdvanced` hint) is the only governance change trigger;
//! the governance-record watch is gone.

use std::collections::BTreeSet;

use rekindle_codec::member_record::head_advanced::RELAYED_SUBKEYS;
use rekindle_governance_runtime::anti_entropy::{self, SeenFrontiers};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::lease::CommunityLeases;
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes;
use rekindle_types::dht_layout::member_record::FRONTIER;
use rekindle_types::id::ChannelId;

use super::HostGovernance;

/// Does a ring change on `subkeys` of a member record rebuild the
/// community? Only a change on the frontier (`M`/3) does. The writer
/// appends to the op-log head (`M`/2) and only then writes the frontier
/// that cites it (`ops::append_own_op`), so the frontier is the commit
/// point a reader can trust, and it is what the rebuild reads
/// (`anti_entropy::inspect_one`); a head change alone triggers nothing
/// (it would read the frontier before it moved). The beat (1) and the
/// outbox (4) go to their one consumer in code today, the `HeadAdvanced`
/// relay (`HostGovernance::relay_head_advanced`); their readers, presence
/// and channel posting, arrive with E3.4 and E3.3, so until then a change
/// on them is only recorded (a debug line), and beats never rebuild
/// (register 2026-10-24, "S8 sequencing"). The card (0), the history ad
/// (5) and the reserved subkeys have no ring consumer.
pub(crate) fn rebuilds_on(subkeys: &[u32]) -> bool {
    subkeys.contains(&FRONTIER)
}

impl HostGovernance {
    /// Handle a watched record's change when it is a ring member record:
    /// `true` when it was one, `false` for any other record. `subkeys` are
    /// the ones the change names; an empty list is a change we did not
    /// observe through our own watch, which is never relayed. The change
    /// is dispatched by subkey ([`rebuilds_on`]): only a frontier change
    /// at a seq not read yet rebuilds the community.
    pub async fn on_watched_change(&self, record: &str, subkeys: &[u32]) -> bool {
        let Some(community) = self.community_of_ring_record(record) else {
            return false;
        };
        // A community we were removed from is read-only history: a late
        // change opens nothing (plan E3.2 S9, R3-1).
        if self.ctx.communities.own_removal(&community).is_some() {
            return true;
        }
        if !subkeys.iter().any(|s| RELAYED_SUBKEYS.contains(s)) {
            tracing::debug!(%community, record, ?subkeys, "ring member record changed, nothing relayed or rebuilt");
            return true;
        }
        let seqs = self.ring_record_seqs(&community, record).await;
        if rebuilds_on(subkeys) {
            let at = seqs
                .as_ref()
                .and_then(|seqs| seqs.get(FRONTIER as usize).copied().flatten());
            match at {
                Some(seq) if self.frontier_unread(&community, record, seq) => {
                    tracing::info!(%community, record, seq, "ring member's frontier moved — rebuilding that community");
                    self.refresh_member_record(&community, record).await;
                }
                Some(seq) => {
                    tracing::debug!(%community, record, seq, "ring member's frontier at a seq already read");
                }
                None => tracing::debug!(%community, record, "ring member's frontier seq not read"),
            }
        } else {
            tracing::debug!(%community, record, ?subkeys, "ring member record changed, no frontier — recorded, not rebuilt");
        }
        if let Some(seqs) = seqs {
            self.relay_head_advanced(&community, record, subkeys, &seqs)
                .await;
        }
        true
    }

    /// The subkey seqs of the ring record `record` of `community`, from
    /// its ring lease.
    async fn ring_record_seqs(&self, community: &str, record: &str) -> Option<Vec<Option<u64>>> {
        let lease = {
            let map = self.ctx.communities.map().read();
            map.get(community)
                .and_then(|c| c.watch_ring.get(record).copied())
        }?;
        self.inspect_dht_record_update_get_seqs(lease).await.ok()
    }

    /// Is `record`'s frontier at `seq` past the last seq caught up from, by
    /// a ring change or a hint (one (record, frontier seq) map,
    /// `CommunityState::head_hints`)? A ring change and a hint for one
    /// frontier write then cost one rebuild.
    pub(super) fn frontier_unread(&self, community: &str, record: &str, seq: u64) -> bool {
        self.ctx
            .communities
            .map()
            .read()
            .get(community)
            .is_some_and(|c| c.head_hints.worth_reading(record, FRONTIER, seq))
    }

    /// Mark `record`'s frontier read at `seq`, once every head it names is
    /// stored (`anti_entropy::inspect_one` counts it read only then), so a
    /// frontier whose head did not fetch yet is read again on its next
    /// change or hint.
    fn mark_frontier_read(&self, community: &str, record: &str, seq: u64) {
        if let Some(c) = self.ctx.communities.map().write().get_mut(community) {
            c.head_hints.record_read(record, FRONTIER, seq);
        }
    }

    /// Read `record`'s frontier, rebuild `community`, and borrow the
    /// channel records the new state names (a ring change or a hint).
    pub(super) async fn refresh_member_record(&self, community: &str, record: &str) {
        if let Some(seq) = self.catch_up_record(community, record).await {
            self.mark_frontier_read(community, record, seq);
        }
        rekindle_governance_runtime::rebuild_governance(self, community).await;
        self.open_new_channel_records(community).await;
    }

    /// Store the heads `record`'s frontier names that we do not hold; the
    /// frontier seq, when every head is stored.
    async fn catch_up_record(&self, community: &str, record: &str) -> Option<u64> {
        let known: Result<BTreeSet<_>, _> = self
            .governance_ops(community)
            .await
            .map(|held| held.iter().map(|op| op.hash).collect());
        let mut known = match known {
            Ok(known) => known,
            Err(error) => {
                tracing::warn!(community, %error, "ring change: op store not read");
                return None;
            }
        };
        let mut seen = SeenFrontiers::new();
        match anti_entropy::inspect_one(self, community, record, &mut seen, &mut known).await {
            Ok(stored) => tracing::debug!(
                community,
                record,
                stored,
                "ring change: frontier heads stored"
            ),
            Err(error) => {
                tracing::warn!(community, record, %error, "ring change: heads not stored");
            }
        }
        seen.get(record).copied()
    }

    /// Borrow every channel record the community's membership names that
    /// it does not hold yet, with our slot writer (channel records share the
    /// registry's slot seed), and hold the leases on the community.
    async fn open_new_channel_records(&self, community: &str) {
        let Ok(transport) = self.transport() else {
            return;
        };
        let named: Vec<(String, String)> = self
            .ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.communities.get(community))
            .map(|m| {
                m.channel_record_keys
                    .iter()
                    .map(|(channel, key)| (channel.clone(), key.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let (opened, writer) = self.ctx.communities.channel_records(community);
        let mut leases = CommunityLeases::default();
        let mut keys = Vec::new();
        for (channel_id, record_key) in named {
            if opened.contains(&record_key) {
                continue;
            }
            let Some(channel) = ChannelId::from_hex(&channel_id) else {
                continue;
            };
            match dht_writes::acquire_str(
                &transport,
                &record_key,
                writer.as_deref(),
                OpenIntent::Hold,
            )
            .await
            {
                Ok(lease) => {
                    leases.channels.insert(channel, lease);
                    keys.push(record_key);
                }
                Err(e) => tracing::debug!(
                    %community, channel_record = %record_key, error = %e,
                    "newly named channel record did not open"
                ),
            }
        }
        if keys.is_empty() {
            return;
        }
        self.ctx.communities.note_channel_records(community, keys);
        // The shell holds and watches its records itself (its leases move
        // at E3.4); the daemon's `SubscriptionManager` watches its own.
        if let Some(shell) = &self.ctx.shell {
            shell.community_records_ready(community, leases).await;
            return;
        }
        let merged = self
            .ctx
            .communities
            .hold_leases(community, leases, |l| dht_writes::key_of(&transport, l));
        for lease in merged.surplus {
            dht_writes::release(&transport, lease).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::dht_layout::member_record::{FRONTIER, SUBKEY_COUNT};

    use super::rebuilds_on;

    /// A ring change on each subkey of `M` rebuilds only for the frontier
    /// (`M`/3), the writer's commit point; the op-log head alone, a beat,
    /// an outbox or any other subkey never does (register 2026-10-24,
    /// "S8 sequencing"). `on_watched_change` itself is driven in the
    /// netsim (`ring_frontier`).
    #[test]
    fn only_a_frontier_change_rebuilds() {
        for subkey in 0..u32::from(SUBKEY_COUNT) {
            assert_eq!(
                rebuilds_on(&[subkey]),
                subkey == FRONTIER,
                "a ring change on subkey {subkey}"
            );
        }
        assert!(
            rebuilds_on(&[2, 3]),
            "a change naming the frontier rebuilds"
        );
        assert!(
            !rebuilds_on(&[1, 2, 4]),
            "a head without its frontier does not"
        );
    }
}
