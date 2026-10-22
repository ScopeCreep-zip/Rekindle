//! The user's input idle time as the Host reads it (plan E2.B): the
//! operating system's (`rekindle_idle::OsIdle`) in production, or a
//! simulated one, so a seeded run fixes when a node goes away.

/// How long the user has given no input.
pub trait IdleSource: Send + Sync {
    /// Seconds since the user's last input; `None` when this session has no
    /// way to tell (no compositor or display reports it). May block on an
    /// OS call: the Host reads it off the async runtime.
    fn idle_seconds(&self) -> Option<u64>;
}
