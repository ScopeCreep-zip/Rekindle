//! The process signals that stop a host process gracefully: `rekindled`
//! (`Host::run_until_stopped`) and the desktop shell, whose session ends
//! by SIGTERM to its scope at logout (`src-tauri/src/stop_signals.rs`).

/// The process signals that stop the process gracefully. systemd stops a
/// unit with SIGTERM (`systemd.kill(5)` `KillSignal=`, default SIGTERM);
/// SIGINT is Ctrl-C in a terminal. All are registered at startup, so a
/// failure to install one stops startup instead of leaving the process
/// stoppable only by SIGKILL. Once registered, tokio keeps the handler
/// for the life of the process, so a second signal never falls back to
/// the default disposition (an immediate kill).
pub struct StopSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    ctrl_c: tokio::signal::windows::CtrlC,
}

impl StopSignals {
    /// Install the handlers. Call from within a tokio runtime.
    ///
    /// # Errors
    /// A handler cannot be installed.
    pub fn register() -> anyhow::Result<Self> {
        let installed = |name: &str, e: std::io::Error| {
            anyhow::anyhow!("cannot install the {name} handler: {e}")
        };
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt()).map_err(|e| installed("SIGINT", e))?,
                terminate: signal(SignalKind::terminate()).map_err(|e| installed("SIGTERM", e))?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                ctrl_c: tokio::signal::windows::ctrl_c().map_err(|e| installed("Ctrl-C", e))?,
            })
        }
    }

    /// The name of the next stop signal received.
    pub async fn next(&mut self) -> &'static str {
        #[cfg(unix)]
        {
            // Biased: two signals at once name SIGINT, not a coin toss;
            // either stops the process the same way.
            tokio::select! {
                biased;
                _ = self.interrupt.recv() => "SIGINT",
                _ = self.terminate.recv() => "SIGTERM",
            }
        }
        #[cfg(windows)]
        {
            self.ctrl_c.recv().await;
            "Ctrl-C"
        }
    }
}
