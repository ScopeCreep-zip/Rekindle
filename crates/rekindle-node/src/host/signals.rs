//! The process signals that stop `rekindled` gracefully.

/// The process signals that stop the daemon gracefully. systemd stops a
/// unit with SIGTERM (`systemd.kill(5)` `KillSignal=`, default SIGTERM);
/// SIGINT is Ctrl-C in a terminal. All are registered at startup, so a
/// failure to install one stops startup instead of leaving the daemon
/// stoppable only by SIGKILL.
pub(super) struct StopSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    ctrl_c: tokio::signal::windows::CtrlC,
}

impl StopSignals {
    pub(super) fn register() -> anyhow::Result<Self> {
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
    pub(super) async fn next(&mut self) -> &'static str {
        #[cfg(unix)]
        {
            tokio::select! {
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
