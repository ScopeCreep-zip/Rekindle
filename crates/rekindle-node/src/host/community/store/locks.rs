//! The per-community locks, rebuild flights and post-rebuild follow-ups
//! of [`CommunityStore`] (plan E3.2 S7, S7l), and the removals that
//! drop them with the community.

use std::collections::BTreeMap;
use std::sync::Arc;

use rekindle_governance_runtime::rebuild_flight::RebuildFlight;
use rekindle_governance_runtime::successor::SuccessorJudgements;
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

/// The per-community single tasks (follow-ups, card refreshes).
type SingleTasks = Arc<parking_lot::Mutex<BTreeMap<String, FollowUp>>>;

/// Mark `community_id`'s task in `tasks` dirty; `true` when none runs.
fn mark(tasks: &SingleTasks, community_id: &str) -> bool {
    let mut tasks = tasks.lock();
    let entry = tasks.entry(community_id.to_string()).or_default();
    entry.dirty = true;
    !std::mem::replace(&mut entry.running, true)
}

/// The running task's next turn: `true` (and clean again) when it was
/// marked since its last turn; otherwise it stops running.
fn take(tasks: &SingleTasks, community_id: &str) -> bool {
    let mut tasks = tasks.lock();
    let Some(entry) = tasks.get_mut(community_id) else {
        return false;
    };
    if std::mem::take(&mut entry.dirty) {
        return true;
    }
    entry.running = false;
    false
}

/// `community_id`'s task did not start: none runs.
fn stop(tasks: &SingleTasks, community_id: &str) {
    if let Some(entry) = tasks.lock().get_mut(community_id) {
        entry.running = false;
    }
}

impl CommunityStore {
    /// The lock that serializes edits to our profile and published card.
    #[must_use]
    pub fn profile_write_lock(&self, community_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        community_lock(&self.profile_writes, community_id)
    }

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

    /// `community_id`'s judged successors, created on first use.
    #[must_use]
    pub fn successor_judgements(&self, community_id: &str) -> Arc<SuccessorJudgements> {
        Arc::clone(
            self.successor_judgements
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
        mark(&self.follow_ups, community_id)
    }

    /// The running follow-up's next turn: `true` (and clean again) when a
    /// rebuild finished since its last turn; otherwise it stops running.
    #[must_use]
    pub fn take_follow_up(&self, community_id: &str) -> bool {
        take(&self.follow_ups, community_id)
    }

    /// `community_id`'s follow-up task did not start: none runs.
    pub fn stop_follow_up(&self, community_id: &str) {
        stop(&self.follow_ups, community_id);
    }

    /// Mark `community_id`'s member card refresh dirty (plan JP1); `true`
    /// when no refresh task runs, so the caller starts one.
    #[must_use]
    pub fn mark_card_refresh(&self, community_id: &str) -> bool {
        mark(&self.card_refreshes, community_id)
    }

    /// The running card refresh's next turn: `true` (and clean again) when
    /// a rebuild asked for one since its last turn; otherwise it stops.
    #[must_use]
    pub fn take_card_refresh(&self, community_id: &str) -> bool {
        take(&self.card_refreshes, community_id)
    }

    /// `community_id`'s card refresh task did not start: none runs.
    pub fn stop_card_refresh(&self, community_id: &str) {
        stop(&self.card_refreshes, community_id);
    }

    /// Forget the per-session single-task markers — on every teardown, both
    /// modes. A follow-up or card refresh runs on its community's scope, a
    /// child of the unlock scope the teardown stops; a follow-up stopped in
    /// its delay returns with `running` still set, and a task the scope
    /// drops never clears it, so a kept marker left the shell's next unlock
    /// (whose store outlives the log-out) never starting either task again.
    /// The locks and rebuild flights stay: a dropped task's guard is released
    /// with it, and a flight's next ticket always exceeds what it built.
    pub fn reset_session_state(&self) {
        self.follow_ups.lock().clear();
        self.card_refreshes.lock().clear();
    }

    /// Drop a community — on leave.
    pub fn remove(&self, community_id: &str) {
        self.map.write().remove(community_id);
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        self.governance_ingest_holds.remove(community_id);
        self.ring_moves.lock().remove(community_id);
        self.rebuilds.lock().remove(community_id);
        self.rebuild_flights.lock().remove(community_id);
        self.op_verify.lock().remove(community_id);
        self.successor_judgements.lock().remove(community_id);
        self.frontier_writes.lock().remove(community_id);
        self.admissions.lock().remove(community_id);
        self.profile_writes.lock().remove(community_id);
        self.follow_ups.lock().remove(community_id);
        self.card_refreshes.lock().remove(community_id);
        self.member_cards
            .write()
            .retain(|(community, _), _| community != community_id);
    }

    /// Drop every community. Called when the session ends.
    pub fn clear(&self) {
        self.map.write().clear();
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        self.governance_ingest_holds.clear();
        self.ring_moves.lock().clear();
        self.rebuilds.lock().clear();
        self.rebuild_flights.lock().clear();
        self.op_verify.lock().clear();
        self.successor_judgements.lock().clear();
        self.frontier_writes.lock().clear();
        self.admissions.lock().clear();
        self.profile_writes.lock().clear();
        self.follow_ups.lock().clear();
        self.card_refreshes.lock().clear();
        self.member_cards.write().clear();
        self.barriers.clear();
    }
}

#[cfg(test)]
#[path = "locks_tests.rs"]
mod tests;
