//! Blocking work a Host starts, through one door (plan E2.B F11).
//!
//! CPU-bound or OS-bound work (an Argon2 passphrase check, an avatar
//! encode, the OS idle read) runs off the async runtime with
//! [`run_blocking`]. Production installs no [`BlockingWatch`], so it is
//! `tokio::task::spawn_blocking`. The network simulator installs one that
//! runs the work inline on a simulation's runtime thread: the runtime
//! polls nothing until it returns, so no task's order depends on how long
//! the work took in wall time, and database calls it makes are the
//! simulation's own (held like any other, `rekindle_netsim::db_queued`).

use std::sync::OnceLock;

/// Decides where blocking work queued on this thread runs.
pub trait BlockingWatch: Send + Sync {
    /// `true` when work queued on the calling thread runs inline on it.
    fn inline(&self) -> bool;
}

static BLOCKING_WATCH: OnceLock<&'static dyn BlockingWatch> = OnceLock::new();

/// Install the process-wide [`BlockingWatch`]. Returns `false` if one is
/// already installed (the first stays).
pub fn watch_blocking(watch: &'static dyn BlockingWatch) -> bool {
    BLOCKING_WATCH.set(watch).is_ok()
}

/// Run `work` off the async runtime and await its result.
///
/// # Errors
///
/// The join error when `work` panicked on a blocking thread.
pub async fn run_blocking<F, R>(work: F) -> Result<R, tokio::task::JoinError>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    if BLOCKING_WATCH.get().is_some_and(|watch| watch.inline()) {
        return Ok(work());
    }
    tokio::task::spawn_blocking(work).await
}

#[cfg(test)]
#[path = "blocking_tests.rs"]
mod tests;
