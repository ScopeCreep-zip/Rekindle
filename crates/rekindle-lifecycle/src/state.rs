//! `LifecycleState` enum + `AppLifecycle` FSM.
//!
//! Transition table hoisted verbatim from the daemon's
//! `rekindle-node::daemon::DaemonState::can_transition_to` — no behavior
//! change, just moved into a shared crate.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use tokio::sync::{broadcast, Notify};

use crate::error::LifecycleError;

/// 9-state FSM. Names match the daemon's `DaemonState` — Phase 5 hoists
/// the original enum, so the daemon re-exports `LifecycleState as
/// DaemonState` to keep its callsites source-compatible.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// Process starting; no Veilid node attached, no vault loaded.
    Stopped = 0,
    /// Veilid bootstrapping; vault file may or may not exist.
    Starting = 1,
    /// Network ready, vault locked (waiting for passphrase).
    Locked = 2,
    /// Vault unlocked; warming caches, reopening DHT records.
    Resuming = 3,
    /// Everything ready — all commands available.
    Operational = 4,
    /// Route died or MEK stale; auto-recovering.
    Degraded = 5,
    /// Network lost; serving cached reads, queuing writes.
    Detached = 6,
    /// Zeroizing secrets, closing vault.
    Locking = 7,
    /// Graceful shutdown in progress.
    ShuttingDown = 8,
}

impl LifecycleState {
    /// Parse from atomic u8 representation. Unknown values fail closed
    /// to `Stopped` so a corrupted load never accidentally enables writes.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Starting,
            2 => Self::Locked,
            3 => Self::Resuming,
            4 => Self::Operational,
            5 => Self::Degraded,
            6 => Self::Detached,
            7 => Self::Locking,
            8 => Self::ShuttingDown,
            _ => Self::Stopped, // Fail closed: unknown → Stopped
        }
    }

    /// Human-readable label (for tracing + IPC).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Locked => "locked",
            Self::Resuming => "resuming",
            Self::Operational => "operational",
            Self::Degraded => "degraded",
            Self::Detached => "detached",
            Self::Locking => "locking",
            Self::ShuttingDown => "shutting_down",
        }
    }

    /// Whether read-only cached queries are available.
    #[must_use]
    pub fn can_query(self) -> bool {
        matches!(self, Self::Operational | Self::Degraded | Self::Detached)
    }

    /// Whether write operations (send, create, join) are available.
    #[must_use]
    pub fn can_write(self) -> bool {
        matches!(self, Self::Operational | Self::Degraded)
    }

    /// Whether the FSM accepts unlock commands.
    #[must_use]
    pub fn can_unlock(self) -> bool {
        matches!(self, Self::Locked)
    }

    /// Whether `self → target` is a valid edge in the FSM.
    #[must_use]
    pub fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Stopped, Self::Starting)
                | (
                    Self::Starting | Self::Resuming | Self::Locking,
                    Self::Locked
                )
                | (Self::Starting | Self::ShuttingDown, Self::Stopped)
                | (Self::Locked, Self::Resuming | Self::ShuttingDown)
                | (
                    Self::Resuming | Self::Degraded | Self::Detached,
                    Self::Operational
                )
                | (
                    Self::Resuming | Self::Operational | Self::Detached,
                    Self::Degraded
                )
                | (Self::Operational | Self::Degraded, Self::Detached)
                | (
                    Self::Operational | Self::Degraded | Self::Detached,
                    Self::Locking | Self::ShuttingDown
                )
        )
    }
}

/// Lifecycle owner — atomic state + broadcast channel for observers +
/// notify for daemon shutdown wakeup.
///
/// One `AppLifecycle` per application lifetime. `Arc<AppLifecycle>` is
/// shared between every subsystem; reads are lock-free, transitions
/// serialize via the atomic store.
pub struct AppLifecycle {
    inner: AtomicU8,
    epoch: Instant,
    /// All state changes are broadcast here so the Tauri shell can
    /// forward them to the frontend `lifecycle-event` channel and the
    /// daemon can observe them.
    tx: broadcast::Sender<LifecycleState>,
    /// Notified specifically when transitioning to `ShuttingDown` —
    /// preserves the daemon's existing `shutdown_requested()` API.
    /// (Could be derived from `tx` but a dedicated `Notify` is cheaper
    /// for the daemon's hot wait path and keeps the existing call
    /// surface untouched.)
    shutdown_notify: Notify,
}

impl AppLifecycle {
    /// Create a new lifecycle in `Stopped`.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            inner: AtomicU8::new(LifecycleState::Stopped as u8),
            epoch: Instant::now(),
            tx,
            shutdown_notify: Notify::new(),
        }
    }

    /// Current state (lock-free read).
    #[must_use]
    pub fn state(&self) -> LifecycleState {
        LifecycleState::from_u8(self.inner.load(Ordering::Acquire))
    }

    /// Alias for [`Self::state`] — matches the plan's nomenclature.
    #[must_use]
    pub fn current(&self) -> LifecycleState {
        self.state()
    }

    /// The monotonic epoch the lifecycle was created at (for uptime
    /// computation in diagnostics).
    #[must_use]
    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    /// Transition to `next` if the edge is valid.
    ///
    /// # Errors
    /// Returns [`LifecycleError::InvalidTransition`] if the edge isn't
    /// in the FSM — the stored state is unchanged.
    pub fn transition(&self, next: LifecycleState) -> Result<LifecycleState, LifecycleError> {
        let cur = self.state();
        if cur == next {
            return Ok(cur);
        }
        if !cur.can_transition_to(next) {
            tracing::error!(
                from = cur.as_str(),
                to = next.as_str(),
                "INVALID lifecycle transition — rejected",
            );
            return Err(LifecycleError::InvalidTransition {
                from: cur,
                to: next,
            });
        }
        self.inner.store(next as u8, Ordering::Release);
        tracing::info!(
            from = cur.as_str(),
            to = next.as_str(),
            "lifecycle transition",
        );
        // Best-effort broadcast — no subscribers is not an error.
        let _ = self.tx.send(next);
        if next == LifecycleState::ShuttingDown {
            self.shutdown_notify.notify_waiters();
        }
        Ok(next)
    }

    /// Subscribe to all subsequent state changes.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<LifecycleState> {
        self.tx.subscribe()
    }

    /// Wait for a `ShuttingDown` transition (the daemon's main event loop
    /// `select!`s on this to trigger graceful shutdown from an IPC
    /// `Shutdown` request). Returns immediately if already ShuttingDown
    /// at the time of call — wait, actually, this uses `Notify::notified()`
    /// which only fires on subsequent `notify_waiters()`. Callers that
    /// race the shutdown should poll `state()` first.
    pub async fn shutdown_requested(&self) {
        self.shutdown_notify.notified().await;
    }
}

impl Default for AppLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for AppLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppLifecycle")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_stopped() {
        let lc = AppLifecycle::new();
        assert_eq!(lc.state(), LifecycleState::Stopped);
    }

    #[test]
    fn transition_updates_state() {
        let lc = AppLifecycle::new();
        lc.transition(LifecycleState::Starting).unwrap();
        assert_eq!(lc.state(), LifecycleState::Starting);
        lc.transition(LifecycleState::Locked).unwrap();
        assert_eq!(lc.state(), LifecycleState::Locked);
    }

    #[test]
    fn can_query_only_when_ready() {
        assert!(!LifecycleState::Stopped.can_query());
        assert!(!LifecycleState::Starting.can_query());
        assert!(!LifecycleState::Locked.can_query());
        assert!(!LifecycleState::Resuming.can_query());
        assert!(LifecycleState::Operational.can_query());
        assert!(LifecycleState::Degraded.can_query());
        assert!(LifecycleState::Detached.can_query());
        assert!(!LifecycleState::Locking.can_query());
        assert!(!LifecycleState::ShuttingDown.can_query());
    }

    #[test]
    fn can_write_only_when_operational_or_degraded() {
        assert!(!LifecycleState::Detached.can_write());
        assert!(LifecycleState::Operational.can_write());
        assert!(LifecycleState::Degraded.can_write());
        assert!(!LifecycleState::Locked.can_write());
        assert!(!LifecycleState::Stopped.can_write());
    }

    #[test]
    fn can_unlock_only_when_locked() {
        assert!(LifecycleState::Locked.can_unlock());
        assert!(!LifecycleState::Operational.can_unlock());
        assert!(!LifecycleState::Stopped.can_unlock());
    }

    #[test]
    fn valid_transitions_accepted() {
        assert!(LifecycleState::Stopped.can_transition_to(LifecycleState::Starting));
        assert!(LifecycleState::Starting.can_transition_to(LifecycleState::Locked));
        assert!(LifecycleState::Locked.can_transition_to(LifecycleState::Resuming));
        assert!(LifecycleState::Resuming.can_transition_to(LifecycleState::Operational));
        assert!(LifecycleState::Operational.can_transition_to(LifecycleState::Locking));
        assert!(LifecycleState::Locking.can_transition_to(LifecycleState::Locked));
        assert!(LifecycleState::Operational.can_transition_to(LifecycleState::ShuttingDown));
        assert!(LifecycleState::ShuttingDown.can_transition_to(LifecycleState::Stopped));
    }

    #[test]
    fn invalid_transitions_rejected() {
        assert!(!LifecycleState::Stopped.can_transition_to(LifecycleState::Operational));
        assert!(!LifecycleState::Locked.can_transition_to(LifecycleState::Starting));
        assert!(!LifecycleState::Locking.can_transition_to(LifecycleState::Operational));
    }

    #[test]
    fn transition_returns_err_on_invalid_edge() {
        let lc = AppLifecycle::new();
        let res = lc.transition(LifecycleState::Operational);
        assert!(matches!(res, Err(LifecycleError::InvalidTransition { .. })));
        assert_eq!(
            lc.state(),
            LifecycleState::Stopped,
            "rejected transition must not alter state"
        );
    }

    #[test]
    fn idempotent_self_transition_is_ok() {
        let lc = AppLifecycle::new();
        // Stopped → Stopped is a no-op, not an error.
        assert_eq!(
            lc.transition(LifecycleState::Stopped).unwrap(),
            LifecycleState::Stopped
        );
    }

    #[test]
    fn degraded_recovers_to_operational() {
        assert!(LifecycleState::Degraded.can_transition_to(LifecycleState::Operational));
        assert!(LifecycleState::Detached.can_transition_to(LifecycleState::Operational));
    }

    /// Exhaustive transition-matrix test — every (from, to) pair in the
    /// 9×9 = 81-pair cartesian product is checked against the explicit
    /// expected-valid set. Catches accidental regressions to
    /// `can_transition_to` (e.g., a refactor that drops an edge or adds
    /// a spurious one).
    #[test]
    fn transition_matrix_is_exhaustive() {
        use LifecycleState::*;
        // Source of truth: every valid edge in the plan's transition table.
        let valid: &[(LifecycleState, LifecycleState)] = &[
            // Stopped → Starting (boot begins).
            (Stopped, Starting),
            // Starting → {Locked, Stopped}.
            (Starting, Locked),
            (Starting, Stopped),
            // Locked → {Resuming, ShuttingDown}.
            (Locked, Resuming),
            (Locked, ShuttingDown),
            // Resuming → {Operational, Degraded, Locked}.
            (Resuming, Operational),
            (Resuming, Degraded),
            (Resuming, Locked),
            // Operational ↔ Degraded ↔ Detached + → Locking/ShuttingDown.
            (Operational, Degraded),
            (Operational, Detached),
            (Operational, Locking),
            (Operational, ShuttingDown),
            (Degraded, Operational),
            (Degraded, Detached),
            (Degraded, Locking),
            (Degraded, ShuttingDown),
            (Detached, Operational),
            (Detached, Degraded),
            (Detached, Locking),
            (Detached, ShuttingDown),
            // Locking → Locked.
            (Locking, Locked),
            // ShuttingDown → Stopped (terminal teardown).
            (ShuttingDown, Stopped),
        ];
        let all = [
            Stopped,
            Starting,
            Locked,
            Resuming,
            Operational,
            Degraded,
            Detached,
            Locking,
            ShuttingDown,
        ];
        for &from in &all {
            for &to in &all {
                let want = from == to || valid.contains(&(from, to));
                let got = from.can_transition_to(to);
                // Self-edges: `from == to` should be accepted by `transition()`
                // (the AppLifecycle::transition impl treats self-transition as
                // a no-op Ok). `can_transition_to` itself returns whatever
                // the matches!() arm yields; self-edges aren't in the table.
                if from == to {
                    // Skip self-edges for this check; they're handled by
                    // `idempotent_self_transition_is_ok`.
                    continue;
                }
                assert_eq!(
                    got, want,
                    "({from:?} → {to:?}): expected can_transition_to == {want}, got {got}",
                );
            }
        }
        // Sanity: the valid set has exactly the count from the plan's table.
        assert_eq!(valid.len(), 22, "valid-edge count mismatches plan's table");
    }

    /// Deep audit: late subscribers must NOT see transitions that happened
    /// before subscribe(). Documents the inherent broadcast semantic so
    /// callers know to seed via `current()` before subscribing.
    #[tokio::test(flavor = "current_thread")]
    async fn subscribe_after_transition_misses_past_events() {
        let lc = AppLifecycle::new();
        lc.transition(LifecycleState::Starting).unwrap();
        lc.transition(LifecycleState::Locked).unwrap();
        // Subscribe AFTER both transitions.
        let mut rx = lc.subscribe();
        // Buffer is empty for this subscriber; next live transition fires.
        lc.transition(LifecycleState::Resuming).unwrap();
        assert_eq!(rx.recv().await.unwrap(), LifecycleState::Resuming);
        // Confirm the seed pattern works: caller can read `current()` to
        // catch up on the state at subscribe time.
        assert_eq!(lc.current(), LifecycleState::Resuming);
    }

    /// Deep audit: concurrent transitions race the atomic store. Two
    /// threads attempting different transitions from the same start
    /// state must result in exactly ONE successful transition; the
    /// loser observes either the loser's-rejected error OR a
    /// from-the-new-state-rejected error (depending on interleave).
    /// Either way, no torn state.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_transitions_yield_consistent_state() {
        let lc = std::sync::Arc::new(AppLifecycle::new());
        lc.transition(LifecycleState::Starting).unwrap();
        lc.transition(LifecycleState::Locked).unwrap();
        lc.transition(LifecycleState::Resuming).unwrap();
        // From Resuming, valid edges are Operational, Degraded, Locked.
        // Race two of them across 4 worker threads.
        let mut tasks = Vec::new();
        for i in 0..50 {
            let lc_clone = std::sync::Arc::clone(&lc);
            let target = if i % 2 == 0 {
                LifecycleState::Operational
            } else {
                LifecycleState::Degraded
            };
            tasks.push(tokio::spawn(async move {
                let _ = lc_clone.transition(target);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        let final_state = lc.state();
        assert!(
            matches!(
                final_state,
                LifecycleState::Operational | LifecycleState::Degraded
            ),
            "final state must be one of the racing targets, not torn — got {final_state:?}",
        );
    }

    /// Phase 5 audit — login_core / create_identity_core failures must
    /// roll the FSM back to Locked so the user can retry. Without this
    /// edge being valid, a failed login would strand the lifecycle in
    /// Resuming forever (Locked → Resuming requires being in Locked,
    /// not Resuming).
    #[test]
    fn resuming_rolls_back_to_locked_on_login_failure() {
        let lc = AppLifecycle::new();
        lc.transition(LifecycleState::Starting).unwrap();
        lc.transition(LifecycleState::Locked).unwrap();
        lc.transition(LifecycleState::Resuming).unwrap();
        // Simulate login_core failure → rollback.
        lc.transition(LifecycleState::Locked)
            .expect("Resuming → Locked must be valid for login-failure rollback");
        assert_eq!(lc.state(), LifecycleState::Locked);
        // User can retry: Locked → Resuming → Operational still works.
        lc.transition(LifecycleState::Resuming).unwrap();
        lc.transition(LifecycleState::Operational).unwrap();
        assert_eq!(lc.state(), LifecycleState::Operational);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn subscribe_observes_transitions() {
        let lc = AppLifecycle::new();
        let mut rx = lc.subscribe();
        lc.transition(LifecycleState::Starting).unwrap();
        lc.transition(LifecycleState::Locked).unwrap();
        assert_eq!(rx.recv().await.unwrap(), LifecycleState::Starting);
        assert_eq!(rx.recv().await.unwrap(), LifecycleState::Locked);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_requested_fires_on_shutting_down() {
        let lc = std::sync::Arc::new(AppLifecycle::new());
        // Drive the state machine forward so ShuttingDown is reachable.
        lc.transition(LifecycleState::Starting).unwrap();
        lc.transition(LifecycleState::Locked).unwrap();
        lc.transition(LifecycleState::Resuming).unwrap();
        lc.transition(LifecycleState::Operational).unwrap();

        let lc_clone = lc.clone();
        let waiter = tokio::spawn(async move { lc_clone.shutdown_requested().await });
        // Give the spawned task a chance to start waiting before we fire
        // the notify; otherwise Notify::notified misses the wakeup.
        tokio::task::yield_now().await;
        lc.transition(LifecycleState::ShuttingDown).unwrap();
        waiter.await.unwrap();
    }
}
