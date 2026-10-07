//! Backoff for a reader waiting on a peer's write: the gossip notification
//! for a message can arrive before its DHT write is readable, so the reader
//! retries the read on this schedule (1 s → 2 s → 4 s), up to
//! [`MAX_RETRIES`] attempts.
//!
//! Writes have no retry here: the record pool's `run_retrying` covers
//! transient errors within one call, and its durable writes hold a missed
//! write and re-push it until it lands (plan C7.13 removed the channel write
//! queue that used to live in this module).

use std::time::Duration;

/// Maximum read attempts before giving up.
pub const MAX_RETRIES: u32 = 3;

/// Backoff duration for a given attempt (0-indexed).
/// 0 → 1s, 1 → 2s, 2 → 4s.
pub fn backoff_duration(attempt: u32) -> Duration {
    Duration::from_secs(1 << attempt.min(4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule() {
        assert_eq!(backoff_duration(0), Duration::from_secs(1));
        assert_eq!(backoff_duration(1), Duration::from_secs(2));
        assert_eq!(backoff_duration(2), Duration::from_secs(4));
    }
}
