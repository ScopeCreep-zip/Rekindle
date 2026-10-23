//! The Host's watch ring (plan E3.2 "Watching", `evidence/slot-integrity.md`
//! §4.5), on both modes: after every rebuild the community's k = 4 ring
//! successors are recomputed from the merged member set
//! (`rekindle_governance_runtime::watch_ring`); a newly assigned member
//! record is opened read-only (which starts warming, V12) and watched on
//! every subkey with `count = None`, and one no longer assigned is
//! released. The record pool is the one watch-death owner (D4): it re-arms
//! a held lease's watch on `count == 0`. A ring record's ValueChange is the
//! governance change trigger ([`HostGovernance::on_watched_change`]).

use std::collections::{BTreeMap, BTreeSet};

use rekindle_governance_runtime::watch_ring::{ring_successors, RING_K};
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes::{self, LeaseId};
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;
use crate::host::community::state::CommunityState;

/// Our pseudonym in `community`, when it is a well-formed key.
fn my_pseudonym(community: &CommunityState) -> Option<PseudonymKey> {
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
    /// changed: at most k per member-set change.
    pub(crate) async fn rearm_watch_ring(&self, community_id: &str) {
        let Ok(node) = self.transport() else {
            return;
        };
        let (wanted, current): (BTreeSet<String>, BTreeSet<String>) = {
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
            (wanted, community.watch_ring.keys().cloned().collect())
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
        let mut opened: BTreeMap<String, LeaseId> = BTreeMap::new();
        for record in wanted.difference(&current) {
            let lease = match dht_writes::acquire_str(&node, record, None, OpenIntent::Check).await {
                Ok(lease) => lease,
                Err(e) => {
                    tracing::debug!(community_id, record, error = %e, "ring record did not open");
                    continue;
                }
            };
            if let Err(e) = dht_writes::watch_all_leased(&node, lease).await {
                tracing::debug!(community_id, record, error = %e, "ring watch not set");
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
        tracing::debug!(community_id, watched = wanted.len(), "watch ring re-armed");
    }

    /// Release `community_id`'s ring watches (leave).
    pub(crate) async fn release_watch_ring(&self, community_id: &str) {
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
