//! Token-bucket rate limiter for per-sender gossip control.

use std::time::Instant;

/// Simple token bucket with refill-on-access semantics.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: u32,
    tokens: f64,
    refill_rate_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a bucket with `capacity` burst and `refill_rate_per_sec`.
    pub fn new(capacity: u32, refill_rate_per_sec: u32) -> Self {
        let now = Instant::now();
        Self {
            capacity,
            tokens: f64::from(capacity),
            refill_rate_per_sec: f64::from(refill_rate_per_sec),
            last_refill: now,
        }
    }

    /// Refill based on the provided timestamp.
    fn refill_at(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_refill);
        let replenished = elapsed.as_secs_f64() * self.refill_rate_per_sec;
        self.tokens = (self.tokens + replenished).min(f64::from(self.capacity));
        self.last_refill = now;
    }

    /// Attempt to consume `amount` tokens at the provided time.
    pub fn try_consume_at(&mut self, amount: u32, now: Instant) -> bool {
        self.refill_at(now);
        let needed = f64::from(amount);
        if self.tokens >= needed {
            self.tokens -= needed;
            return true;
        }
        false
    }

    /// Attempt to consume `amount` tokens using `Instant::now()`.
    pub fn try_consume(&mut self, amount: u32) -> bool {
        self.try_consume_at(amount, Instant::now())
    }

    /// Approximate number of currently available tokens.
    pub fn available(&self) -> f64 {
        self.tokens
    }

    /// Bucket capacity.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Timestamp of the last refill, used by receiver-side limiters
    /// to identify idle buckets for pruning (M10.4).
    pub fn last_refill(&self) -> Instant {
        self.last_refill
    }

    /// Convenience constructor for the default 10 msg/sec channel limit.
    pub fn ten_per_second() -> Self {
        Self::new(10, 10)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::TokenBucket;

    #[test]
    fn token_bucket_sustains_100_msgs_per_second_target() {
        // Architecture §32 / Week 26: gossip throughput target is 100
        // messages/second sustained. The token bucket sized to that
        // rate should accept exactly that many over a one-second window
        // and reject the 101st without burst.
        let mut bucket = TokenBucket::new(100, 100);
        let start = std::time::Instant::now();
        let mut accepted = 0u32;
        for _ in 0..100 {
            if bucket.try_consume_at(1, start) {
                accepted += 1;
            }
        }
        assert_eq!(accepted, 100, "100 msg/sec target must accept full burst");
        assert!(
            !bucket.try_consume_at(1, start),
            "101st msg in same instant must be rejected"
        );
        // After 1s of refill we should be back to full capacity.
        assert!(bucket.try_consume_at(100, start + Duration::from_secs(1)));
    }

    #[test]
    fn token_refill_restores_capacity_over_time() {
        let start = std::time::Instant::now();
        let mut bucket = TokenBucket::new(10, 10);

        assert!(bucket.try_consume_at(10, start));
        assert!(!bucket.try_consume_at(1, start));

        assert!(!bucket.try_consume_at(6, start + Duration::from_millis(500)));
        assert!(bucket.try_consume_at(5, start + Duration::from_millis(500)));

        assert!(bucket.try_consume_at(10, start + Duration::from_secs(2)));
    }
}
