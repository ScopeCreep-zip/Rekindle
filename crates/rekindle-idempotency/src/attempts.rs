//! Bounded proof attempts (plan U1.9): a wrong passphrase costs time, by
//! the escalating delays of Apple's passcode policy (Apple Platform
//! Security, "Escalating time delays for passcode attempts": attempts
//! 1–4 none, 5 → 1 min, 6 → 5 min, 7–8 → 15 min, 9 and on → 1 h). The
//! count is per identity, so a new connection does not reset it, and one
//! limiter serves both unlock (D1) and Protected intent proofs.

use std::collections::HashMap;
use std::hash::Hash;

const MINUTE_MS: u64 = 60 * 1000;

/// The wait after the `failures`-th consecutive failure.
#[must_use]
pub const fn delay_after(failures: u32) -> u64 {
    match failures {
        0..=4 => 0,
        5 => MINUTE_MS,
        6 => 5 * MINUTE_MS,
        7 | 8 => 15 * MINUTE_MS,
        _ => 60 * MINUTE_MS,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Record {
    failures: u32,
    locked_until_ms: u64,
}

/// Consecutive failed proofs per key, and when the key may try again.
#[derive(Debug)]
pub struct AttemptLimiter<K> {
    records: parking_lot::Mutex<HashMap<K, Record>>,
}

impl<K> Default for AttemptLimiter<K> {
    fn default() -> Self {
        Self {
            records: parking_lot::Mutex::new(HashMap::new()),
        }
    }
}

impl<K: Eq + Hash + Clone> AttemptLimiter<K> {
    /// Whether `key` may attempt now.
    ///
    /// # Errors
    /// The key is locked: how long until it may try again, in ms.
    pub fn check(&self, key: &K, now_ms: u64) -> Result<(), u64> {
        match self.records.lock().get(key) {
            Some(r) if r.locked_until_ms > now_ms => Err(r.locked_until_ms - now_ms),
            _ => Ok(()),
        }
    }

    /// Record a failed attempt; returns the wait it imposes, in ms.
    pub fn fail(&self, key: &K, now_ms: u64) -> u64 {
        let mut records = self.records.lock();
        let r = records.entry(key.clone()).or_default();
        r.failures = r.failures.saturating_add(1);
        let wait = delay_after(r.failures);
        r.locked_until_ms = now_ms.saturating_add(wait);
        wait
    }

    /// A successful attempt clears the key.
    pub fn succeed(&self, key: &K) {
        self.records.lock().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_failures_are_free_then_the_waits_escalate() {
        let limiter = AttemptLimiter::<&str>::default();
        for _ in 0..4 {
            assert_eq!(limiter.fail(&"me", 0), 0);
            assert!(limiter.check(&"me", 0).is_ok());
        }
        assert_eq!(limiter.fail(&"me", 0), MINUTE_MS);
        assert_eq!(limiter.check(&"me", 1_000), Err(MINUTE_MS - 1_000));
        assert!(limiter.check(&"me", MINUTE_MS).is_ok());
        assert_eq!(limiter.fail(&"me", MINUTE_MS), 5 * MINUTE_MS);
        assert_eq!(delay_after(8), 15 * MINUTE_MS);
        assert_eq!(delay_after(9), 60 * MINUTE_MS);
        assert!(limiter.check(&"other", 0).is_ok(), "per key");
        limiter.succeed(&"me");
        assert!(limiter.check(&"me", MINUTE_MS).is_ok());
    }
}
