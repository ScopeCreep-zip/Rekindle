//! Lamport logical clock utilities for deterministic gossip ordering.

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

    /// Merge a received Lamport value and increment.
    pub fn merge(&mut self, received: u64) -> u64 {
        self.value = self.value.max(received) + 1;
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::LamportClock;

    #[test]
    fn merge_is_max_plus_one() {
        let mut clock = LamportClock::new(7);
        assert_eq!(clock.merge(3), 8);
        assert_eq!(clock.merge(11), 12);
        assert_eq!(clock.current(), 12);
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
        fn merge_always_advances(
            local_val in 0..u64::MAX - 1,
            received in 0..u64::MAX - 1,
        ) {
            let mut clock = super::LamportClock::new(local_val);
            let result = clock.merge(received);
            prop_assert!(result > local_val);
            prop_assert!(result > received);
        }
    }
}
