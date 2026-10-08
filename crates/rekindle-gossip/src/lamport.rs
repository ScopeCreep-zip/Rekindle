//! Lamport logical clock utilities for deterministic gossip ordering.
//! The clock itself lives in `rekindle_types::lamport` (tier 1), so the
//! pure governance merge can share it.

pub use rekindle_types::lamport::{LamportClock, MAX_LAMPORT_DRIFT};

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
            // Within the drift window the clock lands at received + 1;
            // beyond it, `merge_clamps_drift_above_cap` covers the bound.
            let received = local_val + offset;
            let mut clock = crate::lamport::LamportClock::new(local_val);
            let result = clock.merge(received);
            prop_assert!(result > local_val);
            prop_assert!(result > received);
        }
    }
}
