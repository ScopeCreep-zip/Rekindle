//! The removal watch (plan RM2, E3.2 S9 own removal; D4, ADR 0011), on both
//! modes. Under strong removal a removal concurrent with the remover's own
//! removal is void (`rekindle_governance::merge`), and the op that voids it
//! is written by a member who can remove the remover, possibly after our
//! teardown dropped every watch (`own_removal.rs` `tear_down_removed`). So
//! a torn-down community keeps, for the rest of the session, a watch (never
//! an inspect) on the frontier (`M`/3) of each member record whose member
//! may kick or ban the remover in the merged state. A change there reads
//! every frontier and rebuilds, and a rebuild that no longer removes us
//! restores the community ([`HostGovernance::on_not_removed`]). Veilid
//! delivers each change to a watched subkey (0.5.8
//! `storage_manager/watch_value.rs:2080` `inbound_value_changed`), and the
//! record pool re-arms a dead watch (D4's one retry layer).

use std::collections::{BTreeMap, BTreeSet};

use rekindle_governance::state::GovernanceState;
use rekindle_governance::validate::target::{may_moderate, TargetAction};
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes::{self, LeaseId};
use rekindle_types::dht_layout::member_record::FRONTIER;
use rekindle_types::id::PseudonymKey;

use super::watch_ring::my_pseudonym;
use super::HostGovernance;

/// The member records of everyone in `state` who may remove `remover`
/// (a kick or a ban the validator accepts), never ours nor the remover's.
fn records_of_removers_of(
    state: &GovernanceState,
    remover: &PseudonymKey,
    me: Option<&PseudonymKey>,
) -> BTreeSet<String> {
    state
        .members
        .iter()
        .filter(|(member, _)| *member != remover && Some(*member) != me)
        .filter(|(member, _)| {
            may_moderate(member, remover, TargetAction::Kick, state)
                || may_moderate(member, remover, TargetAction::Ban, state)
        })
        .map(|(_, entry)| entry.member_record.clone())
        .collect()
}

impl HostGovernance {
    /// The torn-down community whose removal watch holds `record`.
    pub(super) fn community_of_removal_record(&self, record: &str) -> Option<String> {
        let map = self.ctx.communities.map().read();
        let mut ids: Vec<&String> = map
            .values()
            .filter(|c| c.removal_watch.contains_key(record))
            .map(|c| &c.id)
            .collect();
        ids.sort_unstable();
        ids.first().map(|id| (*id).clone())
    }

    /// Watch the frontier of every member who may remove our remover in
    /// the merged state, and release the watches of members who no longer
    /// may: called after the teardown and after each rebuild it triggers.
    pub(super) async fn arm_removal_watch(&self, community_id: &str) {
        let Ok(node) = self.transport() else {
            return;
        };
        let lock = self.ctx.communities.ring_move_lock(community_id);
        let _move = lock.lock().await;
        let remover = self
            .ctx
            .communities
            .own_removal(community_id)
            .and_then(|r| r.by);
        let (wanted, current): (BTreeSet<String>, BTreeSet<String>) = {
            let map = self.ctx.communities.map().read();
            let Some(community) = map.get(community_id) else {
                return;
            };
            let me = my_pseudonym(community);
            let wanted = match (remover, &community.governance_state) {
                (Some(remover), Some(state)) => {
                    records_of_removers_of(state, &remover, me.as_ref())
                }
                _ => BTreeSet::new(),
            };
            (wanted, community.removal_watch.keys().cloned().collect())
        };
        let gone: Vec<LeaseId> = {
            let mut map = self.ctx.communities.map().write();
            map.get_mut(community_id).map_or_else(Vec::new, |c| {
                current
                    .difference(&wanted)
                    .filter_map(|record| c.removal_watch.remove(record))
                    .collect()
            })
        };
        for lease in gone {
            dht_writes::release(&node, lease).await;
        }
        let mut opened: BTreeMap<String, LeaseId> = BTreeMap::new();
        for record in wanted.difference(&current) {
            let lease = match dht_writes::acquire_str(&node, record, None, OpenIntent::Check).await
            {
                Ok(lease) => lease,
                Err(e) => {
                    tracing::debug!(community_id, record, error = %e, "removal watch record did not open");
                    continue;
                }
            };
            // A refused watch stays wanted: the pool re-arms it (D4).
            if let Err(e) = dht_writes::watch_leased(&node, lease, &[FRONTIER]).await {
                tracing::debug!(community_id, record, error = %e, "removal watch not yet set");
            }
            opened.insert(record.clone(), lease);
        }
        let surplus: Vec<LeaseId> = {
            let mut map = self.ctx.communities.map().write();
            match map.get_mut(community_id) {
                Some(community) => opened
                    .into_iter()
                    .filter_map(|(record, lease)| {
                        community.removal_watch.insert(record, lease).map(|_| lease)
                    })
                    .collect(),
                None => opened.into_values().collect(),
            }
        };
        for lease in surplus {
            dht_writes::release(&node, lease).await;
        }
        tracing::debug!(community_id, watched = ?wanted, "removal watch armed");
    }

    /// Release the removal watch (the removal was void and the community
    /// comes back up with its own watch ring).
    pub(super) async fn release_removal_watch(&self, community_id: &str) {
        let lock = self.ctx.communities.ring_move_lock(community_id);
        let _move = lock.lock().await;
        let leases: Vec<LeaseId> = self
            .ctx
            .communities
            .map()
            .write()
            .get_mut(community_id)
            .map(|c| std::mem::take(&mut c.removal_watch).into_values().collect())
            .unwrap_or_default();
        if let Ok(node) = self.transport() {
            for lease in leases {
                dht_writes::release(&node, lease).await;
            }
        }
    }

    /// A removal-watch record changed: on a frontier change while we stand
    /// removed, read every frontier and rebuild (which restores the
    /// community when the removal is now void), then follow the merged
    /// state's set of members who may remove the remover.
    pub(super) async fn on_removal_watch_change(&self, community_id: &str, subkeys: &[u32]) {
        if !subkeys.contains(&FRONTIER) || self.ctx.communities.own_removal(community_id).is_none()
        {
            return;
        }
        tracing::info!(community = %community_id, "a remover's frontier changed after our removal — reading frontiers");
        self.catch_up_before_teardown(community_id).await;
        if self.ctx.communities.own_removal(community_id).is_some() {
            self.arm_removal_watch(community_id).await;
        }
    }
}
