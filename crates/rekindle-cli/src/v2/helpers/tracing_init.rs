//! Tracing initialization — rolling daily log file.
//!
//! Logs go to `${XDG_STATE_HOME}/rekindle/logs/rekindle.log`.
//! Never writes to stdout — would corrupt TUI alternate screen buffer.
//! Returns a guard that must be held for the lifetime of the program.

/// Initialize tracing to a rolling daily log file.
pub fn init_tracing() -> tracing_appender::non_blocking::WorkerGuard {
    let log_dir = dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("home directory must exist")
                .join(".local/state")
        })
        .join("rekindle/logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let file_appender = tracing_appender::rolling::daily(log_dir, "rekindle.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rekindle=info,warn".parse().expect("valid filter")),
        )
        .with_ansi(false)
        .init();

    guard
}
