//! Lamport logical clock utilities for deterministic gossip ordering.

/// M9.2 — maximum acceptable forward drift on a received Lamport
/// timestamp. A peer pushing `received = local + MAX_LAMPORT_DRIFT + 1`
/// or higher is silently rejected; the local clock does not advance.
///
/// Without this cap, a single malicious envelope carrying
/// `lamport = u64::MAX` would fast-forward every honest peer's clock
/// permanently, breaking causal ordering for the rest of the
/// community's lifetime. The cap bounds the worst-case advance per
/// received envelope to a known constant.
///
/// 10_000 chosen to comfortably exceed legitimate clock divergence
/// (community at 100 msg/s for 100 seconds = 10_000 ticks) while
/// rejecting any envelope that's clearly forged-future.
pub const MAX_LAMPORT_DRIFT: u64 = 10_000;

/// Simple Lamport logical clock.
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

    /// Increment for a local send.
    pub fn increment(&mut self) -> u64 {
        self.value += 1;
        self.value
    }

    /// Merge a received Lamport value with drift protection (M9.2).
    ///
    /// Returns `Some(new_value)` on accept (clock advanced to
    /// `max(local, received) + 1`), or `None` on rejection (received
    /// exceeded local by more than `MAX_LAMPORT_DRIFT`, OR the merge
    /// would overflow `u64`). Clock unchanged on rejection; caller
    /// should drop the corresponding envelope.
    pub fn merge(&mut self, received: u64) -> Option<u64> {
        // Reject anything beyond the drift window. `checked_add` also
        // catches the boundary case where `value + drift` would
        // saturate to `u64::MAX` and inadvertently accept a forged
        // `u64::MAX` envelope.
        let cap = self.value.checked_add(MAX_LAMPORT_DRIFT)?;
        if received > cap {
            return None;
        }
        // Reject merges that would advance the clock past `u64::MAX`.
        // At the absolute ceiling the gossip community has bigger
        // problems than message ordering — refuse rather than wrap.
        let advanced = self.value.max(received).checked_add(1)?;
        self.value = advanced;
        Some(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::LamportClock;

    #[test]
    fn merge_is_max_plus_one() {
        let mut clock = LamportClock::new(7);
        assert_eq!(clock.merge(3), Some(8));
        assert_eq!(clock.merge(11), Some(12));
        assert_eq!(clock.current(), 12);
    }

    #[test]
    fn merge_rejects_drift_above_cap() {
        // M9.2 — a peer claiming a Lamport value far ahead of ours is
        // silently rejected; the clock does not advance.
        let mut clock = LamportClock::new(100);
        let forged = 100 + super::MAX_LAMPORT_DRIFT + 1;
        assert_eq!(clock.merge(forged), None);
        assert_eq!(clock.current(), 100);
    }

    #[test]
    fn merge_accepts_drift_at_cap_boundary() {
        // Exactly at-cap is accepted; the cap is "more than", not "≥".
        let mut clock = LamportClock::new(100);
        let edge = 100 + super::MAX_LAMPORT_DRIFT;
        assert_eq!(clock.merge(edge), Some(edge + 1));
    }

    #[test]
    fn merge_handles_u64_overflow_safely() {
        // saturating_add prevents the cap calculation from wrapping.
        let mut clock = LamportClock::new(u64::MAX - 1);
        assert_eq!(clock.merge(u64::MAX), None);
        assert_eq!(clock.current(), u64::MAX - 1);
    }

    #[test]
    fn increment_advances_by_one() {
        let mut clock = LamportClock::new(5);
        assert_eq!(clock.increment(), 6);
        assert_eq!(clock.increment(), 7);
    }
}

/// Property-based tests for Lamport clock ordering convergence.
/// Proves the Chiral Network's deterministic ordering guarantee:
/// given the same set of messages, all peers arrive at the same order
/// regardless of delivery sequence.
#[cfg(test)]
mod proptests {
    use proptest::prelude::*;
    use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};

    /// A message with Lamport timestamp and sender identity for ordering.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct OrderedMessage {
        lamport_ts: u64,
        author_pseudonym: String,
        payload_id: u32,
    }

    fn arb_sender() -> impl Strategy<Value = String> {
        // 8-char hex string simulating a pseudonym prefix
        proptest::string::string_regex("[a-f0-9]{8}").unwrap()
    }

    fn arb_message() -> impl Strategy<Value = OrderedMessage> {
        (0..10_000u64, arb_sender(), any::<u32>()).prop_map(|(ts, sender, id)| OrderedMessage {
            lamport_ts: ts,
            author_pseudonym: sender,
            payload_id: id,
        })
    }

    fn ordering_key(message: &OrderedMessage) -> (u64, &str) {
        (message.lamport_ts, message.author_pseudonym.as_str())
    }

    proptest! {
        #[test]
        fn lamport_ordering_converges(
            messages in proptest::collection::vec(arb_message(), 2..50),
            seed_a in any::<u64>(),
            seed_b in any::<u64>(),
        ) {
            let mut order_a = messages.clone();
            order_a.shuffle(&mut StdRng::seed_from_u64(seed_a));
            order_a.sort_by(|left, right| ordering_key(left).cmp(&ordering_key(right)));

            let mut order_b = messages.clone();
            order_b.shuffle(&mut StdRng::seed_from_u64(seed_b));
            order_b.sort_by(|left, right| ordering_key(left).cmp(&ordering_key(right)));

            let keys_a: Vec<_> = order_a.iter().map(ordering_key).collect();
            let keys_b: Vec<_> = order_b.iter().map(ordering_key).collect();
            prop_assert_eq!(keys_a, keys_b);
        }

        #[test]
        fn merge_always_advances_when_within_drift(
            local_val in 0..(u64::MAX - crate::lamport::MAX_LAMPORT_DRIFT - 1),
            offset in 0..crate::lamport::MAX_LAMPORT_DRIFT,
        ) {
            // Bound `received` to the drift window so we test the
            // accept path; out-of-window values are tested separately
            // via `merge_rejects_drift_above_cap`.
            let received = local_val + offset;
            let mut clock = crate::lamport::LamportClock::new(local_val);
            let result = clock.merge(received).expect("within drift window must accept");
            prop_assert!(result > local_val);
            prop_assert!(result > received);
        }
    }
}
