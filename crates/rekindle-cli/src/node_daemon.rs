//! Daemon startup delegation for `rekindle node start`.
//!
//! The CLI is an IPC client. The daemon runtime lives in `rekindle-node`.
//! This module delegates to `rekindle_node::run_daemon()` which owns the
//! process lifecycle: tracing init, path resolution, socket bind, sandbox,
//! accept loop, signal handling, and shutdown.
//!
//! The CLI must NOT initialize tracing before calling `run_daemon()` —
//! the daemon constructs its own reload-capable subscriber for runtime
//! log level adjustment via SIGUSR2.

/// Run the daemon in foreground. Handler for `rekindle node start`.
///
/// Blocks until SIGTERM/Ctrl-C/IPC Shutdown. Returns Ok(()) on clean exit.
pub async fn run_daemon(_attach_timeout: u64) -> anyhow::Result<()> {
    rekindle_node::run_daemon().await
}
