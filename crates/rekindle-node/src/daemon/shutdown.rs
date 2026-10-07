//! Why and when the daemon shuts down.
//!
//! One `CancellationToken` tells every part of the daemon to stop (tokio,
//! *Graceful Shutdown*). The first reason recorded decides the exit status,
//! which is how systemd tells a requested stop from a failure that should
//! restart the daemon from fresh state.

use std::sync::OnceLock;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// How long the bus subscriber waits for in-flight requests to finish once
/// shutdown starts; whatever is still running then is answered 503. A third
/// of the unit's `TimeoutStopSec=15`, which also has to cover the bus drain
/// and the transport shutdown before systemd's SIGKILL.
pub const REQUEST_DRAIN_DEADLINE: Duration = Duration::from_secs(5);

/// How long the bus server waits, once the subscriber has drained, for the
/// daemon connection to close and each client to receive what is queued
/// for it — the reply to `node stop` among them.
pub const BUS_DRAIN_DEADLINE: Duration = Duration::from_secs(2);

/// How long the host waits for its workers once they are told to stop.
/// They stop at their next await, so this bound only catches one stuck in
/// blocking code; it comes out of the same `TimeoutStopSec=15` budget.
pub const WORKER_STOP_DEADLINE: Duration = Duration::from_secs(2);

/// Why the daemon is exiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    /// A stop was asked for: an IPC `Shutdown`, SIGTERM or Ctrl-C.
    Requested,
    /// A request handler panicked. Shared state may be half-mutated, so the
    /// daemon exits for its supervisor to restart it from fresh state.
    HandlerPanic,
    /// The daemon's own bus subscriber could not connect, so nothing
    /// would ever be answered.
    SubscriberFailed,
    /// The identity was destroyed; the daemon restarts with none.
    IdentityDestroyed,
    /// Everything was wiped; the host deletes the Veilid storage once the
    /// transport has stopped, and the daemon restarts empty.
    DataWiped,
}

impl ExitReason {
    /// The process exit status for this reason.
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Requested => 0,
            // A startup failure a restart may fix (systemd `on-failure`).
            Self::SubscriberFailed => 1,
            // sysexits(3) EX_SOFTWARE: "An internal software error has been
            // detected". Non-zero, so systemd's `Restart=on-failure` restarts.
            Self::HandlerPanic => 70,
            // sysexits(3) EX_TEMPFAIL. The unit lists it in
            // `RestartForceExitStatus=`: systemd's `on-failure` would not
            // restart a clean exit (`systemd.service(5)`, Restart= table).
            Self::IdentityDestroyed | Self::DataWiped => 75,
        }
    }
}

/// The daemon-wide shutdown signal.
#[derive(Debug, Default)]
pub struct Shutdown {
    token: CancellationToken,
    reason: OnceLock<ExitReason>,
}

impl Shutdown {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the daemon to shut down. The first reason given is the one the
    /// process exits with; later requests only re-signal.
    pub fn request(&self, reason: ExitReason) {
        if self.reason.set(reason).is_ok() {
            tracing::info!(?reason, "daemon shutdown requested");
        }
        self.token.cancel();
    }

    /// Resolves once shutdown has been requested, including before this was
    /// called.
    pub async fn requested(&self) {
        self.token.cancelled().await;
    }

    /// Run `work` until it finishes or shutdown is requested, whichever is
    /// first; `None` means shutdown won and `work` was dropped.
    pub async fn run_until<T>(&self, work: impl std::future::Future<Output = T>) -> Option<T> {
        tokio::select! {
            () = self.requested() => None,
            value = work => Some(value),
        }
    }

    /// The token behind this signal, for code below the daemon that stops
    /// on a plain `CancellationToken`.
    #[must_use]
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Whether shutdown has been requested.
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.token.is_cancelled()
    }

    /// The reason to exit with: the first one requested, or `Requested` if
    /// the daemon stopped without one.
    #[must_use]
    pub fn reason(&self) -> ExitReason {
        self.reason.get().copied().unwrap_or(ExitReason::Requested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_first_reason_decides_the_exit() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_requested());
        shutdown.request(ExitReason::HandlerPanic);
        shutdown.request(ExitReason::Requested);
        shutdown.requested().await;
        assert_eq!(shutdown.reason(), ExitReason::HandlerPanic);
        assert_eq!(shutdown.reason().exit_code(), 70);
    }

    #[tokio::test]
    async fn run_until_drops_work_at_shutdown() {
        let shutdown = Shutdown::new();
        assert_eq!(shutdown.run_until(async { 7 }).await, Some(7));
        shutdown.request(ExitReason::Requested);
        assert_eq!(shutdown.run_until(std::future::pending::<()>()).await, None);
    }

    #[test]
    fn no_request_exits_cleanly() {
        assert_eq!(Shutdown::new().reason().exit_code(), 0);
    }
}
