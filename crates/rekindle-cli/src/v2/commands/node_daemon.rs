//! Daemon startup delegation for `rekindle node start`.
//!
//! The CLI must NOT initialize tracing before calling `run_daemon()` —
//! the daemon constructs its own reload-capable subscriber for runtime
//! log level adjustment via SIGUSR2.

/// Run the daemon in foreground. Handler for `rekindle node start`.
pub async fn run_daemon(_attach_timeout: u64) -> anyhow::Result<()> {
    rekindle_node::run_daemon().await
}
