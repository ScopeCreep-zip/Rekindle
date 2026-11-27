//! The governance arm of the Host's watch path (plan E3.2 S7, D4), on both
//! modes: a ValueChange on the frontier (`M`/3) of a member record our watch
//! ring holds reads that frontier, stores the heads we lack and, when it
//! stored one, rebuilds that community alone.
//! This (with a `HeadAdvanced` hint) is the only governance change trigger;
//! the governance-record watch is gone.

use std::collections::BTreeSet;

use rekindle_codec::member_record::head_advanced::RELAYED_SUBKEYS;
use rekindle_governance_runtime::anti_entropy::{self, FrontierRead, SeenFrontiers};
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::dht_layout::member_record::{BEAT, CARD, FRONTIER, GOV_HEAD, OUTBOX, ROUTES};

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;

use super::HostGovernance;

/// Does a ring change on `subkeys` of a member record rebuild the
/// community? Only a change on the frontier (`M`/3) does. The writer
/// appends to the op-log head (`M`/2) and only then writes the frontier
/// that cites it (`ops::append_own_op`), so the frontier is the commit
/// point a reader can trust, and it is what the rebuild reads
/// (`anti_entropy::inspect_one`); a head change alone triggers nothing
/// (it would read the frontier before it moved). The outbox (4) goes to
/// the channel outbox reader (plan E3.3, `read_channel_outbox`) and the
/// `HeadAdvanced` relay; the beat (1) to the beat view (plan E3.4 S3,
/// `ring_beat_moved`) and the relay; the routes (6) to the member route
/// cache (plan E3.4 S4, `ring_route_moved`) and the relay; none of them ever rebuilds (register 2026-10-24, "S8 sequencing"). The card (0)
/// goes to the card cache (plan E3.4 "Card", `ring_card_moved`), never
/// relayed; the history ad (5) and the reserved subkeys have no ring consumer.
pub(crate) fn rebuilds_on(subkeys: &[u32]) -> bool {
    subkeys.contains(&FRONTIER)
}

impl HostGovernance {
    /// Handle a watched record's change when it is a ring member record or
    /// a community's registry (`registry_watch`): `true` when it was one,
    /// `false` for any other record. `subkeys` are
    /// the ones the change names; an empty list is a change we did not
    /// observe through our own watch, which is never relayed. The change
    /// is dispatched by subkey ([`rebuilds_on`]): only a frontier change
    /// that names a head we lack rebuilds the community.
    pub async fn on_watched_change(&self, record: &str, subkeys: &[u32]) -> bool {
        if self.join_inbox_changed(record, subkeys) {
            return true;
        }
        if let Some(community) = self.community_of_key_drop(record) {
            if subkeys.is_empty() || subkeys.contains(&super::key_drop::KEY_DROP) {
                self.read_key_drop(&community).await;
            }
            if self.community_of_ring_record(record).is_none() {
                return true;
            }
        }
        if let Some(community) = self.community_of_removal_record(record) {
            self.on_removal_watch_change(&community, subkeys).await;
            return true;
        }
        let Some(community) = self.community_of_ring_record(record) else {
            // A registry (or segment registry) is presence: refreshed,
            // never written (plan E3.4 S5, finding IB1c).
            return self.on_registry_change(record).await;
        };
        // A community we were removed from is read-only history: a late
        // change opens nothing (plan E3.2 S9, R3-1).
        if self.ctx.communities.own_removal(&community).is_some() {
            return true;
        }
        // The card (0) is read by its watchers alone, never relayed (plan
        // E3.4 "Card"; `RELAYED_SUBKEYS` keeps refusing 0).
        if subkeys.contains(&CARD) {
            self.ring_card_moved(&community, record).await;
        }
        if !subkeys.iter().any(|s| RELAYED_SUBKEYS.contains(s)) {
            tracing::debug!(%community, record, ?subkeys, "ring member record changed, nothing relayed or rebuilt");
            return true;
        }
        let seqs = self.ring_record_seqs(&community, record).await;
        let beat_moved = subkeys.contains(&BEAT) && self.ring_beat_moved(&community, record).await;
        if subkeys.contains(&OUTBOX) {
            self.read_channel_outbox(&community, record).await;
        }
        if subkeys.contains(&ROUTES) {
            self.ring_route_moved(&community, record).await;
        }
        let head_seq = seqs
            .as_ref()
            .and_then(|s| s.get(GOV_HEAD as usize).copied().flatten());
        let frontier_follows_head =
            self.frontier_follows_head(&community, record, subkeys, head_seq);
        if rebuilds_on(subkeys) {
            // The change names the frontier: it is force-read (VU3's hint
            // contract), never gated on an inspect a lagging replica answers;
            // the inspected seq only tells a catch-up already in flight
            // whether to read once more. An unknown seq always does.
            let seq = seqs
                .as_ref()
                .and_then(|s| s.get(FRONTIER as usize).copied().flatten())
                .unwrap_or(u64::MAX);
            tracing::debug!(%community, record, seq, "ring member's frontier changed");
            self.refresh_member_record(&community, record, seq, true)
                .await;
        } else {
            tracing::debug!(%community, record, ?subkeys, "ring member record changed, no frontier — recorded, not rebuilt");
        }
        if let Some(seqs) = seqs {
            self.relay_head_advanced(
                &community,
                record,
                subkeys,
                &seqs,
                beat_moved,
                frontier_follows_head,
            );
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

    /// Does this ring change of `record` relay a frontier hint
    /// ([`HeadHints::frontier_follows_head`](crate::host::community::state::HeadHints::frontier_follows_head))?
    fn frontier_follows_head(
        &self,
        community: &str,
        record: &str,
        subkeys: &[u32],
        head_seq: Option<u64>,
    ) -> bool {
        self.ctx
            .communities
            .map()
            .write()
            .get_mut(community)
            .is_some_and(|c| {
                c.head_hints
                    .frontier_follows_head(record, subkeys, head_seq)
            })
    }

    /// `community`'s frontier reads, shared by its sweeps, ring changes and
    /// hints (`HeadHints::frontiers`); a community no longer held has none
    /// to share.
    pub(super) fn seen_frontiers(&self, community: &str) -> SeenFrontiers {
        self.ctx
            .communities
            .map()
            .read()
            .get(community)
            .map(|c| c.head_hints.frontiers())
            .unwrap_or_default()
    }

    /// Claim `record`'s frontier catch-up under the communities write
    /// lock (`HeadHints::claim_frontier`).
    fn claim_frontier(&self, community: &str, record: &str, seq: u64, forced: bool) -> bool {
        self.ctx
            .communities
            .map()
            .write()
            .get_mut(community)
            .is_some_and(|c| c.head_hints.claim_frontier(record, seq, forced))
    }

    /// End the claimed catch-up, marking the frontier read at `read` when
    /// every head it names is stored (`anti_entropy::inspect_one` counts it
    /// read only then), so a frontier whose head did not fetch yet is read
    /// again on its next change or hint; the seq to catch up again.
    fn finish_frontier(&self, community: &str, record: &str, read: Option<u64>) -> Option<u64> {
        self.ctx
            .communities
            .map()
            .write()
            .get_mut(community)
            .and_then(|c| c.head_hints.finish_frontier(record, read))
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
    /// for one frontier write cost one catch-up: whichever claims it first
    /// (`HeadHints::claim_frontier`, `seq` the trigger's, `forced` for a
    /// ring change) reads, and the other is refused; a trigger refused
    /// for a seq above what the read found is caught up after it.
    pub(super) async fn refresh_member_record(
        &self,
        community: &str,
        record: &str,
        seq: u64,
        forced: bool,
    ) {
        if !self.claim_frontier(community, record, seq, forced) {
            tracing::debug!(
                community,
                record,
                seq,
                "member's frontier already read or being read"
            );
            return;
        }
        let mut stored = 0;
        let mut read = None;
        loop {
            tracing::info!(community, record, "reading a member's frontier");
            let (seq_read, stored_now) = self.catch_up_record(community, record).await;
            stored += stored_now;
            read = seq_read.or(read);
            match self.finish_frontier(community, record, seq_read) {
                Some(again) if self.claim_frontier(community, record, again, true) => {}
                _ => break,
            }
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
    /// frontier seq read (this read's when every head is stored), and how
    /// many were stored.
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
        let seen = self.seen_frontiers(community);
        let stored = match anti_entropy::inspect_one(
            self,
            community,
            record,
            &seen,
            &mut known,
            FrontierRead::Hinted,
        )
        .await
        {
            Ok(stored) => stored,
            Err(GovernanceRuntimeError::CommunityNotFound(_)) => {
                tracing::debug!(
                    community,
                    record,
                    "ring change for a community we no longer hold: dropped"
                );
                0
            }
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
        // The shared seq: this read's when it completed, else the last one
        // read (by any path), which a refused trigger is compared against.
        (seen.get(record), stored)
    }
}
