//! Fuzz target: arbitrary arrival orders into ReorderRing.
//!
//! Generates random (seq, value) pairs with random arrival orderings
//! and window sizes. Asserts:
//! - No panic on any input.
//! - Every delivered item is in strictly ascending seq order.
//! - No item within the window is lost (all published items that fit
//!   in the window are eventually delivered after drain_until).
//! - stored_count is consistent with published minus delivered.
//!
//! Run with: cargo test --test reorder_arrivals
//!
//! These are proptest property tests — they generate thousands of random
//! inputs and shrink failures to minimal reproducing cases. This is the
//! adversarial coverage that hand-written permutation tests cannot reach.

#[cfg(all(test, not(loom)))]
mod tests {
    use proptest::prelude::*;
    use rekindle_transport_buff::reorder::ReorderRing;

    /// Generate a random permutation of 0..n.
    fn shuffled_indices(n: usize) -> impl Strategy<Value = Vec<usize>> {
        prop::collection::vec(any::<u8>(), n).prop_map(move |weights| {
            let mut indices: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                let j = weights.get(i).copied().unwrap_or(0) as usize % (i + 1);
                indices.swap(i, j);
            }
            indices
        })
    }

    proptest! {
        /// Arbitrary arrival order, fixed window. Proves in-order delivery.
        #[test]
        fn arbitrary_arrival_order_delivers_in_order(
            window_exp in 0u32..=6,
            n_items in 1usize..=128,
            arrival_order in shuffled_indices(128),
        ) {
            let window = 1usize << window_exp;
            let n = n_items.min(arrival_order.len());
            let ring = ReorderRing::<u64>::new(window);

            // Publish items in the shuffled arrival order.
            let mut published = Vec::new();
            for &idx in &arrival_order[..n] {
                let seq = idx as u64;
                if ring.publish(seq, seq * 10).is_ok() {
                    published.push(seq);
                }
            }

            // Drain contiguous prefix — collect into Vec, assert outside.
            let mut delivered = Vec::new();
            ring.drain_contiguous(|seq, val| {
                delivered.push((seq, val));
            });

            // Verify values match.
            for &(seq, val) in &delivered {
                prop_assert_eq!(val, seq * 10,
                    "value mismatch at seq {}", seq);
            }

            // Delivered items must be in strictly ascending order.
            for w in delivered.windows(2) {
                prop_assert!(w[0].0 < w[1].0,
                    "out of order: {} >= {}", w[0].0, w[1].0);
            }

            // Delivered items must be a contiguous prefix starting at 0.
            for (i, &(seq, _)) in delivered.iter().enumerate() {
                prop_assert_eq!(seq, i as u64,
                    "gap at position {}", i);
            }

            // stored_count must equal published minus delivered.
            let remaining = ring.stored_count();
            let expected_remaining = published.len() - delivered.len();
            prop_assert_eq!(remaining, expected_remaining,
                "stored_count mismatch: remaining={}, published={}, delivered={}",
                remaining, published.len(), delivered.len());
        }

        /// Drain-until forces progress past gaps. No item is lost.
        #[test]
        fn drain_until_never_loses_filled_slots(
            window_exp in 1u32..=5,
            n_items in 1usize..=64,
            arrival_order in shuffled_indices(64),
            drain_target in 1u64..=64,
        ) {
            let window = 1usize << window_exp;
            let n = n_items.min(arrival_order.len());
            let ring = ReorderRing::<u64>::new(window);

            let mut published_seqs = Vec::new();
            for &idx in &arrival_order[..n] {
                let seq = idx as u64;
                if ring.publish(seq, seq).is_ok() {
                    published_seqs.push(seq);
                }
            }

            // drain_contiguous first (delivers the contiguous prefix).
            let mut contiguous = Vec::new();
            ring.drain_contiguous(|seq, _| contiguous.push(seq));

            // drain_until forces past remaining gaps.
            let target = drain_target.min(window as u64);
            let mut forced = Vec::new();
            ring.drain_until(ring.next_deliver().saturating_add(target), |seq, opt| {
                if opt.is_some() {
                    forced.push(seq);
                }
            });

            // Every filled slot that was published and is below next_deliver
            // must have been delivered (via contiguous or forced).
            let all_delivered: Vec<u64> = contiguous.iter()
                .chain(forced.iter())
                .copied()
                .collect();

            for &seq in &published_seqs {
                if seq < ring.next_deliver() {
                    prop_assert!(
                        all_delivered.contains(&seq),
                        "published seq {} below next_deliver {} was lost",
                        seq, ring.next_deliver()
                    );
                }
            }
        }

        /// Rapid fill-drain cycles never corrupt state.
        #[test]
        fn rapid_cycles_no_corruption(
            window_exp in 1u32..=4,
            cycles in 1usize..=50,
        ) {
            let window = 1usize << window_exp;
            let ring = ReorderRing::<u64>::new(window);

            for cycle in 0..cycles {
                let base = cycle as u64 * window as u64;
                for i in 0..window as u64 {
                    ring.publish(base + i, base + i).unwrap();
                }
                let mut delivered = Vec::new();
                ring.drain_contiguous(|seq, val| {
                    delivered.push((seq, val));
                });
                // Assert outside the closure.
                for &(seq, val) in &delivered {
                    prop_assert_eq!(seq, val,
                        "seq/val mismatch in cycle {}", cycle);
                }
                prop_assert_eq!(delivered.len(), window,
                    "wrong count in cycle {}", cycle);
            }
            prop_assert_eq!(ring.stored_count(), 0);
        }
    }
}
