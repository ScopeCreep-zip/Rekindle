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

    /// Await until the FSM accepts an unlock/login — i.e. reaches `Locked`
    /// (Veilid attached). Returns immediately if already unlockable.
    ///
    /// This is Rekindle's analogue of Briar's
    /// `LifecycleManager::waitForStartup()`: the login flow awaits it so it
    /// can't race the asynchronous Veilid attach that drives `Starting →
    /// Locked` (the bug this fixes left the FSM stranded in `Locked` because
    /// the login transitions fired from `Starting`).
    ///
    /// This crate's tokio has only the `sync` feature (no timers), so there
    /// is no internal timeout — callers wrap the call in
    /// `tokio::time::timeout`. The subscribe happens BEFORE the first state
    /// check so a `Starting → Locked` transition landing between the two
    /// cannot be missed (the classic "ready signal already fired" race).
    /// Returns early if the broadcast sender is dropped (app shutting down),
    /// regardless of state.
    pub async fn wait_until_unlockable(&self) {
        let mut rx = self.subscribe();
        if self.state().can_unlock() {
            return;
        }
        loop {
            match rx.recv().await {
                Ok(s) if s.can_unlock() => return,
                // A non-unlockable transition, or a lagged receiver that may
                // have dropped the `Locked` transition — re-read the
                // authoritative state in case we missed it.
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    if self.state().can_unlock() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// Wait for a `ShuttingDown` transition (the daemon's main event loop
    /// `select!`s on this to trigger graceful shutdown from an IPC
    /// `Shutdown` request). Returns immediately if the state is already
    /// `ShuttingDown`.
    ///
    /// `notify_waiters` stores no permit, so the waiter is registered with
    /// `Notified::enable` *before* the state is read: a transition landing
    /// between the two still wakes it (tokio `Notify` docs).
    pub async fn shutdown_requested(&self) {
        let mut notified = std::pin::pin!(self.shutdown_notify.notified());
        notified.as_mut().enable();
        if self.state() == LifecycleState::ShuttingDown {
            return;
        }
        notified.await;
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
mod tests;
