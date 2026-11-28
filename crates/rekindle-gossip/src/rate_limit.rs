//! Token-bucket rate limiter for per-sender gossip control. Time is the
//! caller's monotonic clock (plan C9.2): a host passes
//! `tokio::time::Instant::now().into_std()`, which a paused runtime
//! advances with the simulation.

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
    /// Create a bucket with `capacity` burst and `refill_rate_per_sec`,
    /// full at `now`.
    pub fn new(capacity: u32, refill_rate_per_sec: u32, now: Instant) -> Self {
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
    pub fn ten_per_second(now: Instant) -> Self {
        Self::new(10, 10, now)
    }
}

#[cfg(test)]
#[path = "rate_limit_tests.rs"]
mod tests;
