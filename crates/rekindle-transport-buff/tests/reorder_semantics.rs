#![cfg(not(loom))]
//! Exhaustive reorder semantics tests.
//!
//! Proves the fundamental ordered-delivery property: for every permutation
//! of chunk arrival order, `drain_contiguous` delivers items in seq order.

use rekindle_transport_buff::reorder::{PublishError, ReorderRing};

#[test]
fn single_item_publish_drain() {
    let ring = ReorderRing::<u64>::new(8);
    ring.publish(0, 100).unwrap();
    let mut delivered = Vec::new();
    let count = ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(count, 1);
    assert_eq!(delivered, vec![(0, 100)]);
}

#[test]
fn in_order_delivery() {
    let ring = ReorderRing::<u64>::new(8);
    for i in 0..8 {
        ring.publish(i, i * 10).unwrap();
    }
    let mut delivered = Vec::new();
    let count = ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(count, 8);
    for (i, &(seq, val)) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64);
        assert_eq!(val, i as u64 * 10);
    }
}

#[test]
fn reverse_order_buffers_then_delivers() {
    let ring = ReorderRing::<u64>::new(8);
    // Publish in reverse: 7, 6, 5, ..., 0
    for i in (0..8).rev() {
        ring.publish(i, i * 10).unwrap();
    }
    let mut delivered = Vec::new();
    ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(delivered.len(), 8);
    for (i, &(seq, val)) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64);
        assert_eq!(val, i as u64 * 10);
    }
}

#[test]
fn every_permutation_of_4_delivers_in_order() {
    // 4! = 24 permutations — exhaustive for small N
    let perms = permutations(&[0u64, 1, 2, 3]);
    for perm in &perms {
        let ring = ReorderRing::<u64>::new(8);
        for &seq in perm {
            ring.publish(seq, seq * 10).unwrap();
        }
        let mut delivered = Vec::new();
        ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
        assert_eq!(
            delivered,
            vec![(0, 0), (1, 10), (2, 20), (3, 30)],
            "failed for permutation {:?}",
            perm
        );
    }
}

#[test]
fn gap_holds_later_items() {
    let ring = ReorderRing::<u64>::new(8);
    // Publish 1, 3, 5 — gaps at 0, 2, 4
    ring.publish(1, 10).unwrap();
    ring.publish(3, 30).unwrap();
    ring.publish(5, 50).unwrap();

    // Nothing contiguous from 0
    let mut delivered = Vec::new();
    let count = ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(count, 0);
    assert!(delivered.is_empty());

    // Fill gap at 0
    ring.publish(0, 0).unwrap();
    let count = ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(count, 2); // delivers 0, 1
    assert_eq!(delivered, vec![(0, 0), (1, 10)]);

    // Fill gap at 2
    delivered.clear();
    ring.publish(2, 20).unwrap();
    let count = ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(count, 2); // delivers 2, 3
    assert_eq!(delivered, vec![(2, 20), (3, 30)]);
}

#[test]
fn window_overflow_returns_item() {
    let ring = ReorderRing::<u64>::new(4);
    // next_deliver = 0, window = 4, so valid range is [0, 4)
    let err = ring.publish(4, 999);
    match err {
        Err(PublishError::Overflow { item, .. }) => assert_eq!(item, 999),
        _ => panic!("expected Overflow"),
    }
}

#[test]
fn occupied_returns_item_and_original_survives() {
    let ring = ReorderRing::<u64>::new(8);
    ring.publish(0, 100).unwrap();
    let err = ring.publish(0, 200);
    match err {
        Err(PublishError::Occupied { item }) => assert_eq!(item, 200),
        _ => panic!("expected Occupied"),
    }
    // The original item (100) must still be delivered — the duplicate
    // attempt must not corrupt or replace it.
    let mut delivered = Vec::new();
    ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(delivered, vec![(0, 100)], "original must survive duplicate attempt");
}

#[test]
fn drain_returns_count() {
    let ring = ReorderRing::<u64>::new(8);
    for i in 0..5 {
        ring.publish(i, i).unwrap();
    }
    let count = ring.drain_contiguous(|_, _| {});
    assert_eq!(count, 5);
}

#[test]
fn drain_stops_at_gap() {
    let ring = ReorderRing::<u64>::new(8);
    ring.publish(0, 0).unwrap();
    ring.publish(1, 1).unwrap();
    // gap at 2
    ring.publish(3, 3).unwrap();

    let mut delivered = Vec::new();
    let count = ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(count, 2);
    assert_eq!(delivered, vec![(0, 0), (1, 1)]);
}

#[test]
fn slot_reuse_after_drain() {
    let ring = ReorderRing::<u64>::new(4);
    // First cycle: fill all 4 slots
    for i in 0..4 {
        ring.publish(i, i).unwrap();
    }
    ring.drain_contiguous(|_, _| {});
    assert_eq!(ring.next_deliver(), 4);

    // Second cycle: reuse slots (seq 4..8 map to slots 0..4)
    for i in 4..8 {
        ring.publish(i, i * 10).unwrap();
    }
    let mut delivered = Vec::new();
    ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
    assert_eq!(
        delivered,
        vec![(4, 40), (5, 50), (6, 60), (7, 70)]
    );
}

#[test]
fn window_and_is_empty() {
    let ring = ReorderRing::<u64>::new(16);
    assert_eq!(ring.window(), 16);
    assert!(ring.is_empty());
    ring.publish(0, 0).unwrap();
    assert!(!ring.is_empty());
    ring.drain_contiguous(|_, _| {});
    assert!(ring.is_empty());
}

#[test]
fn next_deliver_advances_correctly() {
    let ring = ReorderRing::<u64>::new(8);
    assert_eq!(ring.next_deliver(), 0);
    ring.publish(0, 0).unwrap();
    ring.publish(1, 1).unwrap();
    ring.drain_contiguous(|_, _| {});
    assert_eq!(ring.next_deliver(), 2);
}

#[test]
fn window_high_tracks_frontier() {
    let ring = ReorderRing::<u64>::new(8);
    assert_eq!(ring.window_high(), 8);
    ring.publish(0, 0).unwrap();
    ring.drain_contiguous(|_, _| {});
    assert_eq!(ring.window_high(), 9); // next_deliver=1, window=8
}

#[test]
fn drop_cleans_filled_slots() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct Counted(u64);
    impl Drop for Counted {
        fn drop(&mut self) {
            // Read the field to prove the correct item is dropped,
            // not just that *some* Drop runs.
            assert!(self.0 < 8, "unexpected value {} in dropped Counted", self.0);
            DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }

    DROP_COUNT.store(0, Ordering::Relaxed);
    {
        let ring = ReorderRing::new(8);
        ring.publish(0, Counted(0)).unwrap();
        ring.publish(1, Counted(1)).unwrap();
        ring.publish(3, Counted(3)).unwrap(); // gap at 2
        // ring drops here — 3 items should be dropped
    }
    assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 3);
}

#[test]
fn many_cycles_stress() {
    let ring = ReorderRing::<u64>::new(8);
    for cycle in 0..100 {
        let base = cycle * 8;
        for i in 0..8 {
            ring.publish(base + i, base + i).unwrap();
        }
        let mut count = 0;
        ring.drain_contiguous(|seq, val| {
            assert_eq!(seq, val);
            count += 1;
        });
        assert_eq!(count, 8);
    }
    assert_eq!(ring.next_deliver(), 800);
}

// ---------------------------------------------------------------------------
// Concurrent multi-producer tests — proves the MP claim
// ---------------------------------------------------------------------------

#[test]
fn concurrent_8_producers_deliver_in_order() {
    use std::sync::Arc;
    use std::thread;

    // 8 producers, each publishes 32 items, window = 256.
    // All 256 items must be delivered in strict seq order.
    let ring = Arc::new(ReorderRing::<u64>::new(256));
    let n_producers = 8usize;
    let items_per = 32u64;

    let handles: Vec<_> = (0..n_producers)
        .map(|p| {
            let r = ring.clone();
            let start = p as u64 * items_per;
            thread::spawn(move || {
                for seq in start..start + items_per {
                    r.publish(seq, seq * 10).unwrap();
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let mut delivered = Vec::new();
    ring.drain_contiguous(|seq, val| delivered.push((seq, val)));

    assert_eq!(
        delivered.len(),
        (n_producers as u64 * items_per) as usize,
        "all items must be delivered"
    );
    for (i, &(seq, val)) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64, "out-of-order at index {i}");
        assert_eq!(val, i as u64 * 10, "wrong value at index {i}");
    }
}

#[test]
fn concurrent_producers_with_consumer_draining() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    // 4 producers publish concurrently while the consumer drains
    // concurrently. This is the production pattern: rayon workers
    // publish, the writer drains.
    let ring = Arc::new(ReorderRing::<u64>::new(256));
    let total = 256u64;
    let done = Arc::new(AtomicBool::new(false));

    let handles: Vec<_> = (0..4)
        .map(|p| {
            let r = ring.clone();
            let start = p * 64;
            thread::spawn(move || {
                for seq in start..start + 64 {
                    while r.publish(seq, seq).is_err() {
                        thread::yield_now();
                    }
                }
            })
        })
        .collect();

    // Consumer drains concurrently.
    let r = ring.clone();
    let d = done.clone();
    let consumer = thread::spawn(move || {
        let mut delivered = Vec::new();
        while (delivered.len() as u64) < total {
            r.drain_contiguous(|seq, val| {
                delivered.push((seq, val));
            });
            if (delivered.len() as u64) < total && !d.load(Ordering::Relaxed) {
                thread::yield_now();
            }
        }
        delivered
    });

    for h in handles {
        h.join().unwrap();
    }
    done.store(true, Ordering::Relaxed);

    let delivered = consumer.join().unwrap();
    assert_eq!(delivered.len(), total as usize);
    // Verify strict seq order.
    for (i, &(seq, val)) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64, "out-of-order at index {i}");
        assert_eq!(val, i as u64, "wrong value at index {i}");
    }
}

// --- Helpers ---

fn permutations(items: &[u64]) -> Vec<Vec<u64>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut result = Vec::new();
    for (i, &item) in items.iter().enumerate() {
        let mut rest: Vec<u64> = items.to_vec();
        rest.remove(i);
        for mut perm in permutations(&rest) {
            perm.insert(0, item);
            result.push(perm);
        }
    }
    result
}
