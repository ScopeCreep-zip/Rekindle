//! Lamport logical clock (CACM 1978) with a bounded receive rule — the one
//! clock every Rekindle Lamport counter is built on.

/// M9.2 — the most a single received Lamport timestamp may advance the
/// local clock. A peer claiming `received > local + MAX_LAMPORT_DRIFT` is
/// still accepted (its message orders by its own timestamp), but the
/// clock only moves to `local + MAX_LAMPORT_DRIFT + 1`.
///
/// Lamport's receive rule (`C := max(C, Tm) + 1`, CACM 1978) has no bound,
/// so one envelope carrying `lamport = u64::MAX` would pin every honest
/// clock at the ceiling. Hybrid Logical Clocks (Kulkarni et al., 2014)
/// bound drift instead; clamping does the same without dropping the
/// message, which matters after a restart, when the local clock may sit
/// far below live traffic until it catches up.
///
/// 10_000 comfortably exceeds legitimate divergence (a community at
/// 100 msg/s for 100 seconds = 10_000 ticks).
pub const MAX_LAMPORT_DRIFT: u64 = 10_000;

/// Why a clock could not produce a timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LamportError {
    /// The clock is at the `u64` ceiling. With every received value
    /// clamped by [`MAX_LAMPORT_DRIFT`], only ~1.8e19 local events reach
    /// it — but reaching it must fail loudly, never wrap.
    Exhausted,
    /// The community whose clock was asked for is not loaded.
    UnknownCommunity,
}

impl std::fmt::Display for LamportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Exhausted => "Lamport clock exhausted (u64 ceiling)",
            Self::UnknownCommunity => "no Lamport clock: community not loaded",
        })
    }
}

impl std::error::Error for LamportError {}

/// Simple Lamport logical clock. Never wraps: at the `u64` ceiling a local
/// increment fails and a merge leaves the clock where it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LamportClock {
    value: u64,
}

impl LamportClock {
    /// Create a clock starting at the provided value.
    pub fn new(value: u64) -> Self {
        Self { value }
    }

    /// Get the current clock value.
    pub fn current(self) -> u64 {
        self.value
    }

    /// Advance for a local event and return the new value; at the `u64`
    /// ceiling the clock is unchanged and the event fails.
    pub fn increment(&mut self) -> Result<u64, LamportError> {
        self.value = self.value.checked_add(1).ok_or(LamportError::Exhausted)?;
        Ok(self.value)
    }

    /// Observe a received Lamport value: the clock becomes
    /// `max(local, min(received, local + MAX_LAMPORT_DRIFT)) + 1`, never
    /// past `u64::MAX`. Returns the new value. The caller keeps the
    /// message whatever it claims; only the clock's advance is bounded.
    pub fn merge(&mut self, received: u64) -> u64 {
        let bounded = received.min(self.value.saturating_add(MAX_LAMPORT_DRIFT));
        if let Some(next) = self.value.max(bounded).checked_add(1) {
            self.value = next;
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::{LamportClock, MAX_LAMPORT_DRIFT};

    #[test]
    fn merge_is_max_plus_one() {
        let mut clock = LamportClock::new(7);
        assert_eq!(clock.merge(3), 8);
        assert_eq!(clock.merge(11), 12);
        assert_eq!(clock.current(), 12);
    }

    #[test]
    fn merge_clamps_drift_above_cap() {
        // M9.2 — a peer claiming a Lamport value far ahead of ours moves
        // the clock by at most MAX_LAMPORT_DRIFT + 1.
        let mut clock = LamportClock::new(100);
        assert_eq!(clock.merge(100 + 1_000_000), 100 + MAX_LAMPORT_DRIFT + 1);
    }

    #[test]
    fn merge_at_cap_boundary() {
        let mut clock = LamportClock::new(100);
        let edge = 100 + MAX_LAMPORT_DRIFT;
        assert_eq!(clock.merge(edge), edge + 1);
    }

    #[test]
    fn forged_u64_max_cannot_pin_the_clock() {
        let mut clock = LamportClock::new(5);
        assert_eq!(clock.merge(u64::MAX), 5 + MAX_LAMPORT_DRIFT + 1);
        // At the ceiling a merge that would pass u64::MAX leaves the clock
        // where it is instead of wrapping.
        let mut high = LamportClock::new(u64::MAX - 1);
        assert_eq!(high.merge(u64::MAX), u64::MAX - 1);
    }

    #[test]
    fn increment_advances_by_one_and_stops_at_the_ceiling() {
        let mut clock = LamportClock::new(5);
        assert_eq!(clock.increment(), Ok(6));
        assert_eq!(clock.increment(), Ok(7));
        let mut top = LamportClock::new(u64::MAX);
        assert_eq!(top.increment(), Err(super::LamportError::Exhausted));
        assert_eq!(top.current(), u64::MAX);
    }
}
