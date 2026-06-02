#![cfg(not(loom))]
//! Adversarial and antagonistic tests.
//!
//! These tests simulate misbehaving producers, resource exhaustion,
//! concurrent contention, and edge cases that would surface regressions
//! in a load-bearing production path.

use rekindle_transport_buff::credit::CreditGuard;
use rekindle_transport_buff::dispatch::DispatchQueue;
use rekindle_transport_buff::pool::SlabPool;
use rekindle_transport_buff::reorder::ReorderRing;
use rekindle_transport_buff::resequencer::{Resequencer, WorkerFailed};
use rekindle_transport_buff::traits::{
    ImmediateReturn, NoOpLifecycle, SlotLifecycle, SpinWake,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// ReorderRing adversarial
// ---------------------------------------------------------------------------

#[test]
fn ring_publish_at_max_u64_seq() {
    // Prove the ring handles near-u64::MAX sequence numbers correctly.
    // new_with_base starts the ring at an arbitrary seq without draining
    // billions of slots. The slot index is seq & mask — the mask truncates
    // to the lower bits regardless of seq magnitude, so the math is correct
    // iff the overflow/wrapping behavior of u64 subtraction in the window
    // check doesn't produce false positives or false negatives.
    let base = u64::MAX - 7;
    let ring = ReorderRing::<u64>::new_with_base(4, base);

    assert_eq!(ring.next_deliver(), base);

    // Publish 4 items at near-MAX seqs.
    // Values are i (not seq*N) to avoid overflow — the point is testing
    // that the ring indexes correctly at high seq, not the value arithmetic.
    for i in 0..4u64 {
        ring.publish(base + i, i + 100).unwrap();
    }

    // Overflow: base + 4 is beyond the window.
    assert!(ring.publish(base + 4, 0).is_err());

    // Drain: all 4 in order with correct seq and value.
    let mut d = Vec::new();
    ring.drain_contiguous(|s, v| d.push((s, v)));
    assert_eq!(d.len(), 4);
    for (i, &(seq, val)) in d.iter().enumerate() {
        assert_eq!(seq, base + i as u64);
        assert_eq!(val, i as u64 + 100);
    }

    assert_eq!(ring.next_deliver(), base + 4);

    // Second cycle after drain: slots reused at base+4..base+7.
    for i in 4..8u64 {
        ring.publish(base + i, i + 200).unwrap();
    }
    let mut d = Vec::new();
    ring.drain_contiguous(|s, v| d.push((s, v)));
    assert_eq!(d.len(), 4);
    for (i, &(seq, val)) in d.iter().enumerate() {
        assert_eq!(seq, base + 4 + i as u64);
        assert_eq!(val, 4 + i as u64 + 200);
    }
}

#[test]
fn ring_rapid_fill_drain_1000_cycles() {
    let ring = ReorderRing::<u64>::new(16);
    for cycle in 0..1000u64 {
        let base = cycle * 16;
        for i in 0..16 {
            ring.publish(base + i, base + i).unwrap();
        }
        let count = ring.drain_contiguous(|s, v| assert_eq!(s, v));
        assert_eq!(count, 16, "cycle {cycle}");
    }
    assert_eq!(ring.stored_count(), 0);
}

#[test]
fn ring_single_slot_window() {
    let ring = ReorderRing::<u64>::new(1);
    ring.publish(0, 42).unwrap();
    // Window is full — next seq overflows.
    assert!(ring.publish(1, 99).is_err());
    ring.drain_contiguous(|s, v| {
        assert_eq!(s, 0);
        assert_eq!(v, 42);
    });
    // Now seq 1 fits.
    ring.publish(1, 99).unwrap();
    let mut d = Vec::new();
    ring.drain_contiguous(|s, v| d.push((s, v)));
    assert_eq!(d, vec![(1, 99)]);
}

#[test]
fn ring_concurrent_producers_large_window() {
    let ring = Arc::new(ReorderRing::<u64>::new(1024));
    let n_producers = 16usize;
    let items_per = 1000u64;
    let total = n_producers as u64 * items_per;

    let handles: Vec<_> = (0..n_producers)
        .map(|p| {
            let r = ring.clone();
            let start = p as u64 * items_per;
            std::thread::spawn(move || {
                for seq in start..start + items_per {
                    while r.publish(seq, seq).is_err() {
                        std::thread::yield_now();
                    }
                }
            })
        })
        .collect();

    // Consumer drains concurrently.
    let mut delivered = 0u64;
    while delivered < total {
        delivered += ring.drain_contiguous(|_, _| {}) as u64;
        if delivered < total {
            std::thread::yield_now();
        }
    }

    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(delivered, total);
    assert_eq!(ring.stored_count(), 0);
}

#[test]
fn ring_drain_until_skips_gaps_correctly() {
    let ring = ReorderRing::<u64>::new(8);

    // Publish 0, 2, 4, 6 (gaps at 1, 3, 5, 7).
    ring.publish(0, 100).unwrap();
    ring.publish(2, 200).unwrap();
    ring.publish(4, 300).unwrap();
    ring.publish(6, 400).unwrap();

    // Drain until 8 — should deliver filled slots and skip gaps.
    let mut filled = Vec::new();
    let mut gaps = Vec::new();
    ring.drain_until(8, |seq, opt| match opt {
        Some(val) => filled.push((seq, val)),
        None => gaps.push(seq),
    });

    assert_eq!(filled, vec![(0, 100), (2, 200), (4, 300), (6, 400)]);
    assert_eq!(gaps, vec![1, 3, 5, 7]);
    assert_eq!(ring.next_deliver(), 8);
    assert_eq!(ring.stored_count(), 0);
}

// ---------------------------------------------------------------------------
// CreditGuard adversarial
// ---------------------------------------------------------------------------

#[test]
fn credit_guard_exact_ceiling_boundary() {
    let guard = CreditGuard::new(100);

    // Reserve exactly at ceiling.
    assert!(guard.try_reserve(100));
    assert!(!guard.try_reserve(1));
    assert_eq!(guard.inflight(), 100);
    assert_eq!(guard.headroom(), 0);

    // Release 1, reserve 1.
    guard.release(1);
    assert!(guard.try_reserve(1));
    assert!(!guard.try_reserve(1));
}

#[test]
fn credit_guard_adversarial_overflow() {
    let guard = CreditGuard::new(u64::MAX);

    // Reserve u64::MAX should succeed (exactly at ceiling).
    assert!(guard.try_reserve(u64::MAX));
    assert!(!guard.try_reserve(1));

    guard.release(u64::MAX);
    assert_eq!(guard.inflight(), 0);
}

#[test]
fn credit_guard_concurrent_exact_admission() {
    // N threads try to reserve 1 byte each against ceiling N.
    // Exactly N must succeed.
    let n = 50usize;
    let guard = Arc::new(CreditGuard::new(n as u64));

    let handles: Vec<_> = (0..n * 2)
        .map(|_| {
            let g = guard.clone();
            std::thread::spawn(move || g.try_reserve(1))
        })
        .collect();

    let successes: usize = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|&ok| ok)
        .count();

    assert_eq!(successes, n);
    assert_eq!(guard.inflight(), n as u64);
}

// ---------------------------------------------------------------------------
// SlabPool adversarial
// ---------------------------------------------------------------------------

#[test]
fn pool_exhaust_release_reacquire_cycle() {
    let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);

    for _ in 0..100 {
        let guards: Vec<_> = (0..4).map(|_| pool.try_acquire().unwrap()).collect();
        assert!(pool.try_acquire().is_none());
        for g in guards {
            pool.release(g);
        }
        pool.reclaim();
        assert_eq!(pool.available(), 4);
    }
}

#[test]
fn pool_guard_drop_on_panic_returns_slab() {
    let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = pool.try_acquire().unwrap();
        panic!("simulated worker panic");
    }));
    assert!(result.is_err());

    // Guard was dropped by panic unwind → slab returned to free.
    assert_eq!(pool.available(), 4);
}

#[test]
fn pool_reset_lifecycle_fires_on_every_return() {
    static RESETS: AtomicUsize = AtomicUsize::new(0);

    struct CountReset;
    impl SlotLifecycle<u64> for CountReset {
        fn construct(&self) -> u64 { 0 }
        fn reset(&self, _: &mut u64) {
            RESETS.fetch_add(1, Ordering::Relaxed);
        }
    }

    RESETS.store(0, Ordering::Relaxed);
    let pool = SlabPool::new(2, ImmediateReturn, CountReset);

    for _ in 0..50 {
        let g = pool.try_acquire().unwrap();
        pool.release(g);
        pool.reclaim();
    }
    // 50 releases + 0 drops = 50 resets via release path.
    assert_eq!(RESETS.load(Ordering::Relaxed), 50);

    for _ in 0..50 {
        let _g = pool.try_acquire().unwrap();
        // dropped without release → reset via Drop path
    }
    assert_eq!(RESETS.load(Ordering::Relaxed), 100);
}

// ---------------------------------------------------------------------------
// DispatchQueue adversarial
// ---------------------------------------------------------------------------

#[test]
fn dispatch_full_returns_all_items() {
    let q = DispatchQueue::<u64>::new(4, SpinWake);

    for i in 0..4 {
        q.try_push(i, i * 10).unwrap();
    }

    // All 4 attempts when full return the item.
    for i in 4..8 {
        let err = q.try_push(i, i * 10).unwrap_err();
        assert_eq!(err, (i, i * 10));
    }

    // Queue still has original 4 items.
    let mut popped = Vec::new();
    while let Some(item) = q.pop() {
        popped.push(item);
    }
    assert_eq!(popped, vec![(0, 0), (1, 10), (2, 20), (3, 30)]);
}

// ---------------------------------------------------------------------------
// Resequencer adversarial
// ---------------------------------------------------------------------------

#[test]
fn resequencer_all_workers_panic() {
    let rs = Resequencer::<u64, u64>::new(4, 4, SpinWake, SpinWake);

    for i in 0..4 {
        rs.submit(i, i * 10).unwrap();
    }

    // All 4 workers panic (drop without complete).
    for _ in 0..4 {
        let _guard = rs.take_work().unwrap();
    }

    // All 4 should drain as WorkerFailed.
    let mut d = Vec::new();
    rs.drain(|seq, result| d.push((seq, result)));
    assert_eq!(d.len(), 4);
    for (i, (seq, result)) in d.iter().enumerate() {
        assert_eq!(*seq, i as u64);
        assert_eq!(*result, Err(WorkerFailed));
    }
}

#[test]
fn resequencer_alternating_success_and_panic() {
    let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);

    for i in 0..6 {
        rs.submit(i, i * 100).unwrap();
    }

    for i in 0..6 {
        let mut guard = rs.take_work().unwrap();
        if i % 2 == 0 {
            // Success.
            let input = guard.take_input().unwrap();
            guard.complete(input + 1).unwrap();
        } else {
            // Panic (drop).
            drop(guard);
        }
    }

    let mut d = Vec::new();
    rs.drain(|seq, result| d.push((seq, result)));
    assert_eq!(d.len(), 6);
    assert_eq!(d[0], (0, Ok(1)));
    assert_eq!(d[1], (1, Err(WorkerFailed)));
    assert_eq!(d[2], (2, Ok(201)));
    assert_eq!(d[3], (3, Err(WorkerFailed)));
    assert_eq!(d[4], (4, Ok(401)));
    assert_eq!(d[5], (5, Err(WorkerFailed)));
}

// ---------------------------------------------------------------------------
// Cross-primitive: credit + dispatch + reorder (the full recv pipeline shape)
// ---------------------------------------------------------------------------

#[test]
fn recv_pipeline_shape_under_memory_pressure() {
    let guard = CreditGuard::new(256); // low ceiling
    let queue = DispatchQueue::<Vec<u8>>::new(8, SpinWake);
    let ring = ReorderRing::<Vec<u8>>::new(16);

    let mut nacked = Vec::new();
    let mut accepted = Vec::new();

    for seq in 0..20u64 {
        let chunk_len = 32u64;
        if guard.try_reserve(chunk_len) {
            // Admitted.
            let data = vec![seq as u8; chunk_len as usize];
            queue.try_push(seq, data.clone()).unwrap();
            // Simulate decrypt worker.
            let (s, d) = queue.pop().unwrap();
            ring.publish(s, d).unwrap();
            accepted.push(seq);
        } else {
            // Memory pressure — shed.
            nacked.push(seq);
        }
    }

    // Drain ring → release credit.
    let mut delivered = Vec::new();
    ring.drain_contiguous(|seq, data| {
        guard.release(data.len() as u64);
        delivered.push(seq);
    });

    // Exactly 8 admitted (256 / 32 = 8 chunks).
    assert_eq!(accepted.len(), 8);
    assert_eq!(nacked.len(), 12);
    assert_eq!(delivered.len(), 8);
    assert_eq!(guard.inflight(), 0);
}
