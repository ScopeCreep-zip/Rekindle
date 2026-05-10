//! Daemon lifecycle state machine.
//!
//! Manages the progression through:
//! STOPPED → STARTING → LOCKED → RESUMING → OPERATIONAL
//! with degraded/detached/locking/shutdown transitions.
//!
//! The daemon state determines which IPC commands are available at any moment.


pub mod dispatch;
pub mod run;

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use tokio::sync::Notify;

/// Daemon lifecycle state.
///
/// Discriminant values are storage identifiers for atomic operations,
/// NOT a readiness ordering. Use the `can_query()`, `can_write()`, and
/// `can_unlock()` methods to check capabilities, not discriminant comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DaemonState {
    /// Daemon not running. No socket, no Veilid node.
    Stopped = 0,
    /// Veilid node bootstrapping. Socket created. Limited commands.
    Starting = 1,
    /// Network ready. Stronghold not unlocked. Secrets not in memory.
    Locked = 2,
    /// Stronghold unlocked. Reopening DHT records, warming MEK cache.
    Resuming = 3,
    /// All systems operational. All commands available.
    Operational = 4,
    /// Route died or MEK stale. Auto-recovering.
    Degraded = 5,
    /// Network lost. Serving cached data, queuing writes.
    Detached = 6,
    /// Zeroizing secrets, closing Stronghold. Transitioning to Locked.
    Locking = 7,
    /// Graceful shutdown in progress.
    ShuttingDown = 8,
}

impl DaemonState {
    /// Parse from atomic u8 representation.
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Starting,
            2 => Self::Locked,
            3 => Self::Resuming,
            4 => Self::Operational,
            5 => Self::Degraded,
            6 => Self::Detached,
            7 => Self::Locking,
            8 => Self::ShuttingDown,
            _ => Self::Stopped, // Fail closed: unknown → Stopped [RC-1]
        }
    }

    /// Human-readable label.
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
        matches!(
            self,
            Self::Operational | Self::Degraded | Self::Detached
        )
    }

    /// Whether write operations (send, create, join) are available.
    #[must_use]
    pub fn can_write(self) -> bool {
        matches!(self, Self::Operational | Self::Degraded)
    }

    /// Whether the daemon accepts unlock commands.
    #[must_use]
    pub fn can_unlock(self) -> bool {
        matches!(self, Self::Locked)
    }

    /// Whether transitioning from `self` to `target` is a valid edge
    /// in the lifecycle state machine.
    #[must_use]
    pub fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Stopped, Self::Starting)
                | (Self::Starting | Self::Resuming | Self::Locking, Self::Locked)
                | (Self::Starting | Self::ShuttingDown, Self::Stopped)
                | (Self::Locked, Self::Resuming | Self::ShuttingDown)
                | (Self::Resuming | Self::Degraded | Self::Detached, Self::Operational)
                | (Self::Resuming | Self::Operational | Self::Detached, Self::Degraded)
                | (Self::Operational | Self::Degraded, Self::Detached)
                | (
                    Self::Operational | Self::Degraded | Self::Detached,
                    Self::Locking | Self::ShuttingDown
                )
        )
    }
}

/// Thread-safe daemon state tracker.
///
/// Uses atomics for lock-free reads from any IPC handler task.
pub struct DaemonLifecycle {
    state: AtomicU8,
    epoch: Instant,
    /// Notified when the daemon transitions to ShuttingDown.
    /// The main event loop `select!`s on this to trigger graceful shutdown
    /// from an IPC `Shutdown` request.
    shutdown_notify: Notify,
}

impl DaemonLifecycle {
    /// Create a new lifecycle tracker in the Stopped state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(DaemonState::Stopped as u8),
            epoch: Instant::now(),
            shutdown_notify: Notify::new(),
        }
    }

    /// Wait for a shutdown signal from an IPC `Shutdown` request.
    ///
    /// Returns when `transition(ShuttingDown)` is called from any task.
    pub async fn shutdown_requested(&self) {
        self.shutdown_notify.notified().await;
    }

    /// Get the current state (lock-free read).
    #[must_use]
    pub fn state(&self) -> DaemonState {
        DaemonState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Transition to a new state if the transition is valid.
    ///
    /// Valid transitions form a directed graph:
    /// - Stopped → Starting
    /// - Starting → Locked, Stopped (startup failure)
    /// - Locked → Resuming, ShuttingDown
    /// - Resuming → Operational, Degraded, Locked (resume failure)
    /// - Operational → Degraded, Detached, Locking, ShuttingDown
    /// - Degraded → Operational, Detached, Locking, ShuttingDown
    /// - Detached → Operational, Degraded, Locking, ShuttingDown
    /// - Locking → Locked
    /// - ShuttingDown → Stopped
    ///
    /// Invalid transitions are logged as errors and ignored.
    pub fn transition(&self, new_state: DaemonState) {
        let old = DaemonState::from_u8(self.state.load(Ordering::Acquire));
        if old == new_state {
            return;
        }

        if !old.can_transition_to(new_state) {
            tracing::error!(
                from = old.as_str(),
                to = new_state.as_str(),
                "INVALID state transition — rejected"
            );
            return;
        }

        self.state.store(new_state as u8, Ordering::Release);
        tracing::info!(
            from = old.as_str(),
            to = new_state.as_str(),
            "daemon state transition"
        );

        if new_state == DaemonState::ShuttingDown {
            self.shutdown_notify.notify_waiters();
        }
    }

    /// The daemon's monotonic epoch (for timestamp generation).
    #[must_use]
    pub fn epoch(&self) -> Instant {
        self.epoch
    }
}

impl Default for DaemonLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_stopped() {
        let lc = DaemonLifecycle::new();
        assert_eq!(lc.state(), DaemonState::Stopped);
    }

    #[test]
    fn transition_updates_state() {
        let lc = DaemonLifecycle::new();
        lc.transition(DaemonState::Starting);
        assert_eq!(lc.state(), DaemonState::Starting);
        lc.transition(DaemonState::Locked);
        assert_eq!(lc.state(), DaemonState::Locked);
    }

    #[test]
    fn can_query_only_when_ready() {
        assert!(!DaemonState::Stopped.can_query());
        assert!(!DaemonState::Starting.can_query());
        assert!(!DaemonState::Locked.can_query());
        assert!(!DaemonState::Resuming.can_query());
        assert!(DaemonState::Operational.can_query());
        assert!(DaemonState::Degraded.can_query());
        assert!(DaemonState::Detached.can_query());
    }

    #[test]
    fn can_write_only_when_operational() {
        assert!(!DaemonState::Detached.can_write());
        assert!(DaemonState::Operational.can_write());
        assert!(DaemonState::Degraded.can_write());
    }

    #[test]
    fn can_unlock_only_when_locked() {
        assert!(DaemonState::Locked.can_unlock());
        assert!(!DaemonState::Operational.can_unlock());
        assert!(!DaemonState::Stopped.can_unlock());
    }

    #[test]
    fn valid_transitions_accepted() {
        assert!(DaemonState::Stopped.can_transition_to(DaemonState::Starting));
        assert!(DaemonState::Starting.can_transition_to(DaemonState::Locked));
        assert!(DaemonState::Locked.can_transition_to(DaemonState::Resuming));
        assert!(DaemonState::Resuming.can_transition_to(DaemonState::Operational));
        assert!(DaemonState::Operational.can_transition_to(DaemonState::Locking));
        assert!(DaemonState::Locking.can_transition_to(DaemonState::Locked));
        assert!(DaemonState::Operational.can_transition_to(DaemonState::ShuttingDown));
        assert!(DaemonState::ShuttingDown.can_transition_to(DaemonState::Stopped));
    }

    #[test]
    fn invalid_transitions_rejected() {
        // Cannot jump from Stopped directly to Operational
        assert!(!DaemonState::Stopped.can_transition_to(DaemonState::Operational));
        // Cannot go backwards from Locked to Starting
        assert!(!DaemonState::Locked.can_transition_to(DaemonState::Starting));
        // Cannot go from Locking to Operational (must go through Locked)
        assert!(!DaemonState::Locking.can_transition_to(DaemonState::Operational));
    }

    #[test]
    fn transition_guards_reject_invalid() {
        let lc = DaemonLifecycle::new();
        // Stopped → Operational is invalid, state should remain Stopped
        lc.transition(DaemonState::Operational);
        assert_eq!(lc.state(), DaemonState::Stopped);
    }

    #[test]
    fn degraded_recovers_to_operational() {
        assert!(DaemonState::Degraded.can_transition_to(DaemonState::Operational));
        assert!(DaemonState::Detached.can_transition_to(DaemonState::Operational));
    }
}
