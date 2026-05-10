//! UNIX signal handling for the daemon process.
//!
//! Handles: SIGTERM (terminate), SIGINT (interrupt/Ctrl-C), SIGHUP (reload),
//! SIGUSR1 (diagnostic dump), SIGUSR2 (log level rotation).
//!
//! Each signal maps to a specific daemon action. The main select! loop
//! receives signals from SignalStream and dispatches to the appropriate handler.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use tracing_subscriber::reload;
use tracing_subscriber::EnvFilter;

/// Daemon-relevant signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// SIGTERM: graceful shutdown.
    Terminate,
    /// SIGINT (Ctrl-C): same as Terminate.
    Interrupt,
    /// SIGHUP: reload configuration without restart.
    HangUp,
    /// SIGUSR1: write diagnostic dump to file.
    User1,
    /// SIGUSR2: rotate tracing log level.
    User2,
}

/// Multiplexed signal stream for use in tokio::select!.
pub struct SignalStream {
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigint: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sighup: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigusr1: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigusr2: tokio::signal::unix::Signal,
}

impl SignalStream {
    /// Create a new signal stream, registering handlers for all relevant signals.
    pub fn new() -> anyhow::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Ok(Self {
                sigterm: signal(SignalKind::terminate())?,
                sigint: signal(SignalKind::interrupt())?,
                sighup: signal(SignalKind::hangup())?,
                sigusr1: signal(SignalKind::user_defined1())?,
                sigusr2: signal(SignalKind::user_defined2())?,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    /// Wait for the next signal. Returns None if the signal infrastructure is broken.
    pub async fn next(&mut self) -> Option<Signal> {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.sigterm.recv() => Some(Signal::Terminate),
                _ = self.sigint.recv() => Some(Signal::Interrupt),
                _ = self.sighup.recv() => Some(Signal::HangUp),
                _ = self.sigusr1.recv() => Some(Signal::User1),
                _ = self.sigusr2.recv() => Some(Signal::User2),
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.ok()?;
            Some(Signal::Interrupt)
        }
    }
}

// ── Log level rotation ──────────────────────────────────────────────────

/// Log levels in rotation order.
const LEVEL_CYCLE: &[&str] = &["info", "debug", "trace", "warn", "error"];

/// Current position in the level cycle.
static LEVEL_INDEX: AtomicU8 = AtomicU8::new(0);

/// Global reload handle stored during tracing initialization.
/// Type-erased because the full generic type of reload::Handle depends
/// on the subscriber stack which is constructed at init time.
static RELOAD_HANDLE: OnceLock<ReloadHandle> = OnceLock::new();

/// Type-erased reload handle that can swap EnvFilter at runtime.
pub struct ReloadHandle {
    inner: Box<dyn ReloadFn>,
}

trait ReloadFn: Send + Sync {
    fn reload(&self, filter: EnvFilter) -> Result<(), String>;
}

impl<S> ReloadFn for reload::Handle<EnvFilter, S>
where
    S: tracing::Subscriber + 'static,
{
    fn reload(&self, filter: EnvFilter) -> Result<(), String> {
        reload::Handle::reload(self, filter).map_err(|e| format!("{e}"))
    }
}

/// Store the reload handle during tracing initialization.
/// Called exactly once from `run/tracing.rs::init_tracing()`.
pub fn register_reload_handle<S>(handle: reload::Handle<EnvFilter, S>)
where
    S: tracing::Subscriber + 'static,
{
    let boxed = ReloadHandle {
        inner: Box::new(handle),
    };
    if RELOAD_HANDLE.set(boxed).is_err() {
        tracing::error!("reload handle registered twice — this is a bug");
    }
}

/// Rotate the tracing log level through a fixed cycle.
///
/// Cycle: INFO → DEBUG → TRACE → WARN → ERROR → INFO
///
/// Called on SIGUSR2. Atomically swaps the active EnvFilter on the
/// tracing subscriber. Takes effect immediately for all future events.
/// No restart required. No lock contention — the reload layer uses
/// an internal ArcSwap.
pub fn rotate_log_level() {
    let Some(handle) = RELOAD_HANDLE.get() else {
        tracing::error!(
            "SIGUSR2: cannot rotate log level — reload handle not registered. \
             Tracing must be initialized via daemon::run::tracing::init_tracing()"
        );
        return;
    };

    let next_idx = LEVEL_INDEX.fetch_add(1, Ordering::Relaxed) % u8::try_from(LEVEL_CYCLE.len()).unwrap_or(1);
    let level = LEVEL_CYCLE[next_idx as usize % LEVEL_CYCLE.len()];

    let filter = match EnvFilter::try_new(level) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(level, error = %e, "SIGUSR2: failed to construct EnvFilter");
            return;
        }
    };

    match handle.inner.reload(filter) {
        Ok(()) => {
            // Log at the NEW level so the operator sees confirmation regardless
            // of what the new level is.
            tracing::warn!(level, "log level rotated");
        }
        Err(e) => {
            tracing::error!(error = %e, "SIGUSR2: filter reload failed");
        }
    }
}
