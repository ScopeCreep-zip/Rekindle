use parking_lot::RwLock;
use std::collections::HashMap;

/// Rate-limit state: tracks message timestamps per (channel, sender).
pub struct RateLimiter {
    /// Map of (channel_id, sender_pseudonym) -> recent message timestamps (Unix seconds)
    window: RwLock<HashMap<(String, String), Vec<i64>>>,
    /// Max messages allowed in the window
    max_messages: usize,
    /// Window duration in seconds (stored as i64 for timestamp arithmetic)
    window_seconds: i64,
}

impl RateLimiter {
    pub fn new(max_messages: u32, window_seconds: u32) -> Self {
        Self {
            window: RwLock::new(HashMap::new()),
            max_messages: max_messages as usize,
            window_seconds: i64::from(window_seconds),
        }
    }

    /// Check if a message should be rate-limited. Returns `true` if the message
    /// is allowed, `false` if the sender has exceeded the rate limit.
    pub fn check_and_record(&self, channel_id: &str, sender: &str, now: i64) -> bool {
        let key = (channel_id.to_string(), sender.to_string());
        let mut window = self.window.write();
        let timestamps = window.entry(key).or_default();

        // Remove timestamps outside the window
        let cutoff = now - self.window_seconds;
        timestamps.retain(|&t| t > cutoff);

        if timestamps.len() >= self.max_messages {
            return false; // Rate limited
        }

        timestamps.push(now);
        true
    }

    /// Periodically clean up stale entries to prevent unbounded memory growth.
    pub fn cleanup(&self, now: i64) {
        let cutoff = now - self.window_seconds;
        let mut window = self.window.write();
        window.retain(|_, timestamps| {
            timestamps.retain(|&t| t > cutoff);
            !timestamps.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_messages_under_limit() {
        let limiter = RateLimiter::new(3, 10);
        assert!(limiter.check_and_record("ch1", "user1", 100));
        assert!(limiter.check_and_record("ch1", "user1", 101));
        assert!(limiter.check_and_record("ch1", "user1", 102));
    }

    #[test]
    fn blocks_messages_over_limit() {
        let limiter = RateLimiter::new(3, 10);
        assert!(limiter.check_and_record("ch1", "user1", 100));
        assert!(limiter.check_and_record("ch1", "user1", 101));
        assert!(limiter.check_and_record("ch1", "user1", 102));
        // 4th message within the window should be blocked
        assert!(!limiter.check_and_record("ch1", "user1", 103));
    }

    #[test]
    fn allows_after_window_expires() {
        let limiter = RateLimiter::new(3, 10);
        assert!(limiter.check_and_record("ch1", "user1", 100));
        assert!(limiter.check_and_record("ch1", "user1", 101));
        assert!(limiter.check_and_record("ch1", "user1", 102));
        // After window expires, should allow again
        assert!(limiter.check_and_record("ch1", "user1", 111));
    }

    #[test]
    fn separate_channels_have_independent_limits() {
        let limiter = RateLimiter::new(2, 10);
        assert!(limiter.check_and_record("ch1", "user1", 100));
        assert!(limiter.check_and_record("ch1", "user1", 101));
        assert!(!limiter.check_and_record("ch1", "user1", 102));
        // Different channel should still be allowed
        assert!(limiter.check_and_record("ch2", "user1", 102));
    }

    #[test]
    fn separate_senders_have_independent_limits() {
        let limiter = RateLimiter::new(2, 10);
        assert!(limiter.check_and_record("ch1", "user1", 100));
        assert!(limiter.check_and_record("ch1", "user1", 101));
        assert!(!limiter.check_and_record("ch1", "user1", 102));
        // Different sender should still be allowed
        assert!(limiter.check_and_record("ch1", "user2", 102));
    }

    #[test]
    fn cleanup_removes_stale_entries() {
        let limiter = RateLimiter::new(3, 10);
        limiter.check_and_record("ch1", "user1", 100);
        limiter.check_and_record("ch1", "user1", 101);
        // After cleanup at time 115, entries from 100 and 101 should be gone
        limiter.cleanup(115);
        let window = limiter.window.read();
        assert!(window.is_empty());
    }
}
