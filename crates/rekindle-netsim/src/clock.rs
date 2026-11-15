//! Simulated time and randomness (plan C9.3): a node's wall clock follows
//! the paused tokio clock from a fixed epoch, plus that node's skew; its
//! generator is seeded per node from the run seed.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use rekindle_utils::{Clock, RandomSource};

/// The simulated epoch: 2027-01-15T08:00:00Z, so every run starts at the
/// same wall time.
pub const SIM_EPOCH_MS: u64 = 1_800_000_000_000;

/// A node's wall clock: [`SIM_EPOCH_MS`] plus the tokio clock's elapsed
/// time since the run started, plus the node's skew.
#[derive(Debug)]
pub struct SimClock {
    start: tokio::time::Instant,
    skew_ms: AtomicI64,
    /// The reading held while the node sleeps, 0 when awake
    /// ([`crate::Fault::Sleep`]).
    frozen_ms: AtomicU64,
}

impl SimClock {
    /// A clock starting now on the (paused) tokio clock.
    #[must_use]
    pub fn new(start: tokio::time::Instant) -> Self {
        Self {
            start,
            skew_ms: AtomicI64::new(0),
            frozen_ms: AtomicU64::new(0),
        }
    }

    /// Set this node's skew from the simulated true time.
    pub fn set_skew_ms(&self, skew_ms: i64) {
        self.skew_ms.store(skew_ms, Ordering::SeqCst);
    }

    /// The node's process is suspended (`asleep`): the clock reads the
    /// instant of the suspension until it resumes, then the true time, so
    /// the first reading after the wake jumps across the whole sleep.
    pub fn freeze(&self, asleep: bool) {
        let held = if asleep { self.running_ms() } else { 0 };
        self.frozen_ms.store(held, Ordering::SeqCst);
    }

    fn running_ms(&self) -> u64 {
        let elapsed = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        SIM_EPOCH_MS
            .saturating_add(elapsed)
            .saturating_add_signed(self.skew_ms.load(Ordering::SeqCst))
    }
}

impl Clock for SimClock {
    fn now_ms(&self) -> u64 {
        match self.frozen_ms.load(Ordering::SeqCst) {
            0 => self.running_ms(),
            held => held,
        }
    }
}

/// A node's generator: `StdRng` (ChaCha, a `CryptoRng`) seeded from the run
/// seed and the node's index.
#[derive(Debug)]
pub struct SimRandom(Mutex<StdRng>);

impl SimRandom {
    /// The generator of node `index` in run `seed`.
    #[must_use]
    pub fn new(seed: u64, index: usize) -> Self {
        let mut material = [0u8; 32];
        material[..8].copy_from_slice(&seed.to_le_bytes());
        material[8..16].copy_from_slice(&(index as u64).to_le_bytes());
        Self(Mutex::new(StdRng::from_seed(
            *blake3::hash(&material).as_bytes(),
        )))
    }
}

impl rand::CryptoRng for SimRandom {}

impl RandomSource for SimRandom {
    fn fill(&self, dest: &mut [u8]) {
        self.0.lock().fill_bytes(dest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_and_index_fix_the_stream() {
        let take = |seed, index| {
            let rng = SimRandom::new(seed, index);
            (rng.next_u32(), rng.next_u32())
        };
        assert_eq!(take(7, 0), take(7, 0));
        assert_ne!(take(7, 0), take(7, 1));
        assert_ne!(take(7, 0), take(8, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn the_clock_follows_the_paused_runtime_and_its_skew() {
        let clock = SimClock::new(tokio::time::Instant::now());
        assert_eq!(clock.now_ms(), SIM_EPOCH_MS);
        tokio::time::advance(std::time::Duration::from_secs(90)).await;
        assert_eq!(clock.now_secs(), SIM_EPOCH_MS / 1000 + 90);
        clock.set_skew_ms(-3_600_000);
        assert_eq!(clock.now_ms(), SIM_EPOCH_MS + 90_000 - 3_600_000);
    }

    #[tokio::test(start_paused = true)]
    async fn a_sleeping_clock_stands_still_then_jumps_across_the_sleep() {
        let clock = SimClock::new(tokio::time::Instant::now());
        clock.freeze(true);
        tokio::time::advance(std::time::Duration::from_secs(600)).await;
        assert_eq!(clock.now_ms(), SIM_EPOCH_MS, "asleep, no time passes");
        clock.freeze(false);
        assert_eq!(clock.now_ms(), SIM_EPOCH_MS + 600_000, "the wake's jump");
    }
}
