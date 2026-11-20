//! The per-community locks, rebuild flights and post-rebuild follow-ups
//! of [`CommunityStore`] (plan E3.2 S7, S7l), and the removals that
//! drop them with the community.

use std::collections::BTreeMap;
use std::sync::Arc;

use rekindle_governance_runtime::rebuild_flight::RebuildFlight;
use rekindle_governance_runtime::verify_cache::OpVerifyCache;

use super::CommunityStore;

/// One community's post-rebuild follow-up: whether its task runs, and
/// whether a rebuild finished since the task last read the state.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct FollowUp {
    running: bool,
    dirty: bool,
}

pub(super) type CommunityLocks =
    Arc<parking_lot::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>>;

/// `community_id`'s lock in `locks`, created on first use.
fn community_lock(locks: &CommunityLocks, community_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    Arc::clone(locks.lock().entry(community_id.to_string()).or_default())
}

impl CommunityStore {
    /// The lock that serializes `community_id`'s watch-ring moves.
    #[must_use]
    pub fn ring_move_lock(&self, community_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        community_lock(&self.ring_moves, community_id)
    }

    /// The lock that serializes `community_id`'s governance rebuilds.
    #[must_use]
    pub fn rebuild_lock(&self, community_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        community_lock(&self.rebuilds, community_id)
    }

    /// `community_id`'s rebuild single flight.
    #[must_use]
    pub fn rebuild_flight(&self, community_id: &str) -> Arc<RebuildFlight> {
        Arc::clone(
            self.rebuild_flights
                .lock()
                .entry(community_id.to_string())
                .or_default(),
        )
    }

    /// `community_id`'s kept op verdicts, created on first use.
    #[must_use]
    pub fn op_verify_cache(&self, community_id: &str) -> Arc<OpVerifyCache> {
        Arc::clone(
            self.op_verify
                .lock()
                .entry(community_id.to_string())
                .or_default(),
        )
    }

    /// The lock that serializes `community_id`'s `Frontier` writes.
    #[must_use]
    pub fn frontier_write_lock(&self, community_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        community_lock(&self.frontier_writes, community_id)
    }

    /// The lock that serializes `community_id`'s bootstrap admissions (plan
    /// SU1).
    #[must_use]
    pub fn admission_lock(&self, community_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        community_lock(&self.admissions, community_id)
    }

    /// Mark `community_id`'s post-rebuild follow-up dirty; `true` when no
    /// follow-up task runs, so the caller starts one.
    #[must_use]
    pub fn mark_follow_up(&self, community_id: &str) -> bool {
        let mut follow_ups = self.follow_ups.lock();
        let entry = follow_ups.entry(community_id.to_string()).or_default();
        entry.dirty = true;
        !std::mem::replace(&mut entry.running, true)
    }

    /// The running follow-up's next turn: `true` (and clean again) when a
    /// rebuild finished since its last turn; otherwise it stops running.
    #[must_use]
    pub fn take_follow_up(&self, community_id: &str) -> bool {
        let mut follow_ups = self.follow_ups.lock();
        let Some(entry) = follow_ups.get_mut(community_id) else {
            return false;
        };
        if std::mem::take(&mut entry.dirty) {
            return true;
        }
        entry.running = false;
        false
    }

    /// `community_id`'s follow-up task did not start: none runs.
    pub fn stop_follow_up(&self, community_id: &str) {
        if let Some(entry) = self.follow_ups.lock().get_mut(community_id) {
            entry.running = false;
        }
    }

    /// Drop a community — on leave.
    pub fn remove(&self, community_id: &str) {
        self.map.write().remove(community_id);
        self.ring_moves.lock().remove(community_id);
        self.rebuilds.lock().remove(community_id);
        self.rebuild_flights.lock().remove(community_id);
        self.op_verify.lock().remove(community_id);
        self.frontier_writes.lock().remove(community_id);
        self.admissions.lock().remove(community_id);
        self.follow_ups.lock().remove(community_id);
        self.member_cards
            .write()
            .retain(|(community, _), _| community != community_id);
    }

    /// Drop every community. Called when the session ends.
    pub fn clear(&self) {
        self.map.write().clear();
        self.ring_moves.lock().clear();
        self.rebuilds.lock().clear();
        self.rebuild_flights.lock().clear();
        self.op_verify.lock().clear();
        self.frontier_writes.lock().clear();
        self.admissions.lock().clear();
        self.follow_ups.lock().clear();
        self.member_cards.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use rekindle_records::lease::LeaseId;

    use crate::host::community::state::CommunityState;
    use crate::host::community::store::{new_map, CommunityStore};

    fn store_with(community: &str) -> CommunityStore {
        let store = CommunityStore::new(new_map());
        store.map().write().insert(
            community.to_owned(),
            CommunityState {
                id: community.to_owned(),
                ..CommunityState::default()
            },
        );
        store
    }

    /// The rebuilds that finish while a follow-up runs cost it one more
    /// turn; it stops once a turn finds nothing new.
    #[test]
    fn follow_ups_coalesce_into_one_task_and_one_more_turn() {
        let store = store_with("c");
        assert!(
            store.mark_follow_up("c"),
            "the first rebuild starts the task"
        );
        assert!(!store.mark_follow_up("c"), "a second rebuild only marks it");
        assert!(!store.mark_follow_up("c"));
        assert!(store.take_follow_up("c"), "one turn for both");
        assert!(!store.take_follow_up("c"), "then the task stops");
        assert!(store.mark_follow_up("c"), "and the next rebuild starts one");
    }

    /// Leave takes the watch ring's leases with the community's own, so a
    /// ring move that finished after `release_watch_ring` leaks none.
    #[test]
    fn take_leases_takes_the_watch_ring() {
        let store = store_with("c");
        store
            .map()
            .write()
            .get_mut("c")
            .expect("community")
            .watch_ring
            .insert("record".to_owned(), LeaseId(7));
        let (_, ring) = store.take_leases("c");
        assert_eq!(ring, vec![LeaseId(7)]);
        assert!(store.map().read()["c"].watch_ring.is_empty());
    }

    /// Leave drops the community's locks and follow-up state with it.
    #[test]
    fn remove_prunes_the_per_community_state() {
        let store = store_with("c");
        assert!(store.mark_follow_up("c"));
        let before = store.frontier_write_lock("c");
        let verdicts = store.op_verify_cache("c");
        store.remove("c");
        assert!(!std::sync::Arc::ptr_eq(
            &verdicts,
            &store.op_verify_cache("c")
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &before,
            &store.frontier_write_lock("c")
        ));
        assert!(store.mark_follow_up("c"), "no follow-up survives the leave");
    }
}
