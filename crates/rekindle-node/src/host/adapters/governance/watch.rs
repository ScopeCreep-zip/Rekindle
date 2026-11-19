//! The governance arm of the Host's watch path (plan E3.2 S7, D4), on both
//! modes: a ValueChange on the frontier (`M`/3) of a member record our watch
//! ring holds reads that frontier, stores the heads we lack and, when it
//! stored one, rebuilds that community alone.
//! This (with a `HeadAdvanced` hint) is the only governance change trigger;
//! the governance-record watch is gone.

use std::collections::BTreeSet;

use rekindle_codec::member_record::head_advanced::RELAYED_SUBKEYS;
use rekindle_governance_runtime::anti_entropy::{self, FrontierRead, SeenFrontiers};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::lease::CommunityLeases;
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes;
use rekindle_types::dht_layout::member_record::{FRONTIER, OUTBOX};
use rekindle_types::id::ChannelId;

use super::HostGovernance;

/// Does a ring change on `subkeys` of a member record rebuild the
/// community? Only a change on the frontier (`M`/3) does. The writer
/// appends to the op-log head (`M`/2) and only then writes the frontier
/// that cites it (`ops::append_own_op`), so the frontier is the commit
/// point a reader can trust, and it is what the rebuild reads
/// (`anti_entropy::inspect_one`); a head change alone triggers nothing
/// (it would read the frontier before it moved). The outbox (4) goes to
/// the channel outbox reader (plan E3.3, `read_channel_outbox`) and the
/// `HeadAdvanced` relay; the beat (1) only to the relay until E3.4's
/// presence reader, so a change on it is only recorded, and neither ever
/// rebuilds (register 2026-10-24, "S8 sequencing"). The card (0), the history ad
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
    /// that names a head we lack rebuilds the community.
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
        if subkeys.contains(&OUTBOX) {
            self.read_channel_outbox(&community, record).await;
        }
        if rebuilds_on(subkeys) {
            // The change names the frontier: it is force-read (VU3's hint
            // contract), never gated on an inspect a lagging replica answers.
            tracing::info!(%community, record, "ring member's frontier changed — reading it");
            self.refresh_member_record(&community, record).await;
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

    /// Mark `record`'s frontier read at `seq`, once every head it names is
    /// stored (`anti_entropy::inspect_one` counts it read only then), so a
    /// frontier whose head did not fetch yet is read again on its next
    /// change or hint; a hint for a frontier seq already read is not read
    /// again (`head_advanced.rs`, `head_hints.worth_reading`).
    fn mark_frontier_read(&self, community: &str, record: &str, seq: u64) {
        if let Some(c) = self.ctx.communities.map().write().get_mut(community) {
            c.head_hints.record_read(record, FRONTIER, seq);
        }
    }

    /// Read `record`'s frontier and, when the read stored a head we lacked,
    /// rebuild `community` and borrow the channel records the new state
    /// names (a ring change or a hint). The rebuild merges the stored heads
    /// (the `anti_entropy` module contract, as the periodic sweep does in
    /// `anti_entropy_loop`); a frontier that stored nothing, at any seq,
    /// adds nothing to merge, since our state is the merge of every op we
    /// hold, and the read's own D15 convergence check
    /// (`anti_entropy::check_convergence`) rebuilds when equal heads carry
    /// a roster checksum other than ours (R5-3). A ring change and a hint
    /// for one frontier write cost one rebuild: the second read finds
    /// every head held.
    pub(super) async fn refresh_member_record(&self, community: &str, record: &str) {
        let (read, stored) = self.catch_up_record(community, record).await;
        if let Some(seq) = read {
            self.mark_frontier_read(community, record, seq);
        }
        if stored == 0 {
            tracing::debug!(
                community,
                record,
                ?read,
                "ring member's frontier names no head we lack — not rebuilt"
            );
            return;
        }
        tracing::info!(
            community,
            record,
            stored,
            "ring member's frontier moved — rebuilding that community"
        );
        rekindle_governance_runtime::rebuild_governance(self, community).await;
    }

    /// Store the heads `record`'s frontier names that we do not hold; the
    /// frontier seq, when every head is stored, and how many were stored.
    async fn catch_up_record(&self, community: &str, record: &str) -> (Option<u64>, usize) {
        let known: Result<BTreeSet<_>, _> = self
            .governance_ops(community)
            .await
            .map(|held| held.iter().map(|op| op.hash).collect());
        let mut known = match known {
            Ok(known) => known,
            Err(error) => {
                tracing::warn!(community, %error, "ring change: op store not read");
                return (None, 0);
            }
        };
        let mut seen = SeenFrontiers::new();
        let stored = match anti_entropy::inspect_one(
            self,
            community,
            record,
            &mut seen,
            &mut known,
            FrontierRead::Hinted,
        )
        .await
        {
            Ok(stored) => stored,
            Err(error) => {
                tracing::warn!(community, record, %error, "ring change: heads not stored");
                0
            }
        };
        tracing::debug!(
            community,
            record,
            stored,
            "ring change: frontier heads stored"
        );
        (seen.get(record).copied(), stored)
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
