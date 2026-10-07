//! Liveness of the daemon's bus subscriber, for the systemd watchdog.
//!
//! The subscriber loop beats every [`Heartbeat::tick`] from an interval arm
//! of its `select!`, so it beats while idle and stops beating only when the
//! loop itself is stuck. The watchdog pings systemd only while the last
//! beat is fresh, and reports a stale one with `WATCHDOG=trigger`, so a
//! stuck subscriber gets the process restarted instead of answering nothing
//! forever.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The beat interval outside a systemd watchdog, where only the subscriber
/// loop itself reads it.
pub const DEFAULT_TICK: Duration = Duration::from_secs(10);

/// The subscriber loop's last beat, as milliseconds since `epoch`.
pub struct Heartbeat {
    epoch: Instant,
    last_ms: AtomicU64,
    tick: Duration,
}

impl Heartbeat {
    /// A heartbeat beating every `tick` that counts as having just beaten,
    /// so the watchdog keeps pinging while the subscriber connects.
    #[must_use]
    pub fn new(tick: Duration) -> Self {
        Self {
            epoch: Instant::now(),
            last_ms: AtomicU64::new(0),
            tick,
        }
    }

    /// How often the subscriber loop beats.
    #[must_use]
    pub fn tick(&self) -> Duration {
        self.tick
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Record a beat now.
    pub fn beat(&self) {
        self.last_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Whether the last beat is recent enough to vouch for the loop: within
    /// two ticks, so one late tick is not a stall.
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        let age = self
            .now_ms()
            .saturating_sub(self.last_ms.load(Ordering::Relaxed));
        Duration::from_millis(age) < self.tick * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_until_a_beat_is_missed() {
        let tick = Duration::from_secs(1);
        let heartbeat = Heartbeat::new(tick);
        assert!(heartbeat.is_fresh());
        heartbeat.beat();
        assert!(heartbeat.is_fresh());
        let stale = Heartbeat {
            epoch: Instant::now()
                .checked_sub(tick * 3)
                .expect("monotonic clock covers three ticks"),
            last_ms: AtomicU64::new(0),
            tick,
        };
        assert!(!stale.is_fresh());
        stale.beat();
        assert!(stale.is_fresh());
    }
}
