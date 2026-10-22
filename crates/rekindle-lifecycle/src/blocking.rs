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
mod tests {
    use super::*;

    struct Inline;
    impl BlockingWatch for Inline {
        fn inline(&self) -> bool {
            std::thread::current().name() == Some("inline-here")
        }
    }

    #[test]
    fn a_watched_thread_runs_the_work_inline_and_others_spawn() {
        static WATCH: Inline = Inline;
        assert!(watch_blocking(&WATCH));
        assert!(!watch_blocking(&WATCH), "the first watch stays");
        let run = |name: &str| {
            std::thread::Builder::new()
                .name(name.to_string())
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .unwrap();
                    let caller = std::thread::current().id();
                    rt.block_on(run_blocking(move || std::thread::current().id() == caller))
                        .unwrap()
                })
                .unwrap()
                .join()
                .unwrap()
        };
        assert!(run("inline-here"), "inline on the watched thread");
        assert!(!run("elsewhere"), "a blocking thread otherwise");
    }
}
