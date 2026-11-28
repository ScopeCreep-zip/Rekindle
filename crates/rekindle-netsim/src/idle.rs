//! Simulated input idle time (plan E2.B): a node's idle port reads the time
//! since the scenario last set it idle, on that node's [`SimClock`], so a
//! seed, not this machine, fixes when it goes away. A node starts active.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rekindle_utils::{Clock, IdleSource};

use crate::clock::SimClock;

/// `last_input_ms` while the user is active (input on every tick).
const ACTIVE: u64 = u64::MAX;

/// A node's idle port: the user's last input on the node's clock, or
/// active.
#[derive(Debug)]
pub struct SimIdle {
    clock: Arc<SimClock>,
    last_input_ms: AtomicU64,
}

impl SimIdle {
    /// An active user on `clock`.
    #[must_use]
    pub fn new(clock: Arc<SimClock>) -> Self {
        Self {
            clock,
            last_input_ms: AtomicU64::new(ACTIVE),
        }
    }

    /// The user's last input is now: idle from here on.
    pub fn set_idle(&self) {
        self.last_input_ms
            .store(self.clock.now_ms(), Ordering::SeqCst);
    }

    /// The user is active again (input on every tick).
    pub fn set_active(&self) {
        self.last_input_ms.store(ACTIVE, Ordering::SeqCst);
    }
}

impl IdleSource for SimIdle {
    fn idle_seconds(&self) -> Option<u64> {
        let last = self.last_input_ms.load(Ordering::SeqCst);
        if last == ACTIVE {
            return Some(0);
        }
        Some(self.clock.now_ms().saturating_sub(last) / 1000)
    }
}

#[cfg(test)]
#[path = "idle_tests.rs"]
mod tests;
