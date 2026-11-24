//! The Host's watch ring (plan E3.2 "Watching", `evidence/slot-integrity.md`
//! §4.5), on both modes: after every rebuild the community's k = 4 ring
//! successors are recomputed from the merged member set
//! (`rekindle_governance_runtime::watch_ring`); a newly assigned member
//! record is opened read-only (which starts warming, V12) and watched on
//! every subkey with `count = None`, and one no longer assigned is
//! released. A record that did not open is left out of the ring, so the
//! next move tries it again; the anti-entropy tick makes one move per
//! community, so a failed open waits at most one sweep period. The record
//! pool is the one watch retry layer (D4): it re-arms a held lease's watch
//! on `count == 0` and one Veilid refused. A ring record's ValueChange is the
//! governance change trigger ([`HostGovernance::on_watched_change`]).

use std::collections::{BTreeMap, BTreeSet};

use rekindle_governance_runtime::watch_ring::{ring_successors, RING_K};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes::{self, LeaseId};
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;
use crate::host::community::state::CommunityState;

/// Our pseudonym in `community`, when it is a well-formed key.
pub(super) fn my_pseudonym(community: &CommunityState) -> Option<PseudonymKey> {
    let bytes = hex::decode(community.my_pseudonym_key.as_deref()?).ok()?;
    Some(PseudonymKey(bytes.try_into().ok()?))
}

/// The member records `community` keeps warm beyond its held leases: our
/// own `M` and our ring successors' (ADR 0011, plan E3.2 record warming),
/// in key order. The anchor `A` is the community's governance lease.
#[must_use]
pub fn warm_member_records(community: &CommunityState) -> Vec<String> {
    let mut keys: BTreeSet<String> = community.watch_ring.keys().cloned().collect();
    if let (Some(me), Some(state)) = (my_pseudonym(community), &community.governance_state) {
        if let Some(own) = state.members.get(&me) {
            keys.insert(own.member_record.clone());
        }
    }
    keys.into_iter().collect()
}

impl HostGovernance {
    /// The community whose ring holds `record`.
    pub(super) fn community_of_ring_record(&self, record: &str) -> Option<String> {
        let map = self.ctx.communities.map().read();
        let mut ids: Vec<&String> = map
            .values()
            .filter(|c| c.watch_ring.contains_key(record))
            .map(|c| &c.id)
            .collect();
        ids.sort_unstable();
        ids.first().map(|id| (*id).clone())
    }

    /// Recompute `community_id`'s ring successors and move the watches that
    /// changed: at most k per member-set change. Moves are serialized per
    /// community, and each reads the member set once it holds the lock, so
    /// the move after the last merge leaves exactly that set's successors.
    pub(crate) async fn rearm_watch_ring(&self, community_id: &str) {
        let lock = self.ctx.communities.ring_move_lock(community_id);
        let _move = lock.lock().await;
        self.rearm_watch_ring_locked(community_id).await;
    }

    /// [`Self::rearm_watch_ring`] with the community's ring move lock
    /// already held by the caller.
    pub(super) async fn rearm_watch_ring_locked(&self, community_id: &str) {
        let Ok(node) = self.transport() else {
            return;
        };
        let (wanted, current, members): (BTreeSet<String>, BTreeSet<String>, usize) = {
            let map = self.ctx.communities.map().read();
            let Some(community) = map.get(community_id) else {
                return;
            };
            let wanted = match (my_pseudonym(community), &community.governance_state) {
                (Some(me), Some(state)) => ring_successors(community_id, state, &me, RING_K)
                    .into_iter()
                    .collect(),
                _ => BTreeSet::new(),
            };
            let members = community
                .governance_state
                .as_ref()
                .map_or(0, |s| s.members.len());
            (
                wanted,
                community.watch_ring.keys().cloned().collect(),
                members,
            )
        };
        let mut gone: Vec<LeaseId> = Vec::new();
        {
            let mut map = self.ctx.communities.map().write();
            if let Some(community) = map.get_mut(community_id) {
                for record in current.difference(&wanted) {
                    gone.extend(community.watch_ring.remove(record));
                }
            }
        }
        for lease in gone {
            dht_writes::release(&node, lease).await;
        }
        // A round with no answer is retried on every ring change (plan
        // E3.2 S7d-keys.8).
        if wanted != current {
            self.ctx.communities.unlock_barriers().poke(community_id);
        }
        let mut opened: BTreeMap<String, LeaseId> = BTreeMap::new();
        for record in wanted.difference(&current) {
            let lease = match dht_writes::acquire_str(&node, record, None, OpenIntent::Check).await
            {
                Ok(lease) => lease,
                Err(e) => {
                    tracing::debug!(community_id, record, error = %e, "ring record did not open");
                    continue;
                }
            };
            // A watch Veilid refused stays wanted: the pool records it as
            // set only once Veilid accepts it and re-arms it itself (D4).
            if let Err(e) = dht_writes::watch_all_leased(&node, lease).await {
                tracing::debug!(community_id, record, error = %e, "ring watch not yet set");
            }
            opened.insert(record.clone(), lease);
        }
        let surplus: Vec<LeaseId> = {
            let mut map = self.ctx.communities.map().write();
            match map.get_mut(community_id) {
                Some(community) => opened
                    .into_iter()
                    .filter_map(|(record, lease)| match community.watch_ring.entry(record) {
                        std::collections::btree_map::Entry::Occupied(_) => Some(lease),
                        std::collections::btree_map::Entry::Vacant(slot) => {
                            slot.insert(lease);
                            None
                        }
                    })
                    .collect(),
                None => opened.into_values().collect(),
            }
        };
        for lease in surplus {
            dht_writes::release(&node, lease).await;
        }
        self.spawn_cold_beat_reads(community_id, wanted.difference(&current).cloned().collect());
        tracing::debug!(community_id, members, watched = ?wanted, "watch ring re-armed");
    }

    /// Read the beat of each record newly in the ring, cold (plan E3.4 S3:
    /// the view holds a ring member's status before its next change; with
    /// no arrival only the beat number counts, so an old beat shows its
    /// member offline). Off the ring move lock.
    fn spawn_cold_beat_reads(&self, community_id: &str, records: Vec<String>) {
        if records.is_empty() {
            return;
        }
        let host = self.clone();
        let community = community_id.to_owned();
        self.scope().spawn_or_drop("ring beat read", async move {
            for record in records {
                let lease = {
                    let map = host.ctx.communities.map().read();
                    map.get(&community)
                        .and_then(|c| c.watch_ring.get(&record).copied())
                };
                if let Some(lease) = lease {
                    host.read_member_beat(&community, &record, lease, None)
                        .await;
                    // Its routes (`M`/6) into the member route cache (plan
                    // E3.4 S4): a ring member's later changes arrive by
                    // the watch.
                    host.read_member_routes(&community, &record, lease).await;
                }
            }
        });
    }

    /// Release `community_id`'s ring watches (leave).
    pub(crate) async fn release_watch_ring(&self, community_id: &str) {
        let lock = self.ctx.communities.ring_move_lock(community_id);
        let _move = lock.lock().await;
        let leases: Vec<LeaseId> = self
            .ctx
            .communities
            .map()
            .write()
            .get_mut(community_id)
            .map(|c| std::mem::take(&mut c.watch_ring).into_values().collect())
            .unwrap_or_default();
        if let Ok(node) = self.transport() {
            for lease in leases {
                dht_writes::release(&node, lease).await;
            }
        }
    }
}
