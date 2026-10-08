//! File logging for a frontend: every line through the identifier
//! scrubber (`rekindle_utils::log_scrub`), never to stdout (the TUI owns
//! the terminal).

use std::path::PathBuf;

use crate::ClientError;

/// Log to `<data root logs>/<file>` with daily rotation: the directory the
/// desktop and `rekindled` log to (`rekindle_utils::paths::DataRoot::logs`).
/// The guard flushes on drop and must live as long as the process.
///
/// # Errors
/// No home directory is known, or the log directory cannot be created.
pub fn init(file: &str) -> Result<tracing_appender::non_blocking::WorkerGuard, ClientError> {
    let log_dir: PathBuf = rekindle_utils::paths::DataRoot::resolve()
        .map_err(|e| ClientError::Log(e.to_string()))?
        .logs;
    std::fs::create_dir_all(&log_dir)
        .map_err(|e| ClientError::Log(format!("cannot create {}: {e}", log_dir.display())))?;
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(log_dir, file));
    tracing_subscriber::fmt()
        .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(writer))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rekindle=info,warn")),
        )
        .with_ansi(false)
        .init();
    Ok(guard)
}
