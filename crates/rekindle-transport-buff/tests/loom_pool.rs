//! Loom model-checking tests for [`SlabPool`].
//!
//! Run with `RUSTFLAGS="--cfg loom" cargo test --test loom_pool`.
//!
//! # What these models prove
//!
//! 1. Two threads acquiring from the same pool never get the same slab
//!    index — the ArrayQueue free-list's MPMC pop is linearizable.
//!
//! 2. A released slab (returned to the free list via guard Drop) is
//!    available for reacquire by another thread — the push/pop ordering
//!    on the ArrayQueue is correct.
//!
//! 3. The pool's capacity invariant holds under concurrent acquire/release:
//!    `free.len() + pending.len() + in_use == capacity` at quiescence.
//!
//! # Why these models are small
//!
//! `SlabPool` uses `crossbeam_queue::ArrayQueue` for its free list, which
//! is already loom-verified in crossbeam's own test suite. These models
//! verify the *pool's* use of the queue — the acquire/release/reclaim
//! lifecycle — not the queue itself.

#![cfg(loom)]

use loom::sync::Arc;
use loom::thread;

use rekindle_transport_buff::{ImmediateReturn, NoOpLifecycle, SlabPool};

/// Model 1: Two threads acquire from a pool of 2. Each must get a
/// distinct index. Proves the free-list pop is linearizable.
#[test]
fn concurrent_acquire_distinct_indices() {
    loom::model(|| {
        let pool = Arc::new(SlabPool::<u64>::new(2, ImmediateReturn, NoOpLifecycle));

        let p1 = pool.clone();
        let t1 = thread::spawn(move || p1.try_acquire().map(|g| g.index()));

        let p2 = pool.clone();
        let t2 = thread::spawn(move || p2.try_acquire().map(|g| g.index()));

        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();

        // Both must succeed (pool has 2 slabs, 2 acquires).
        let i1 = r1.expect("first acquire must succeed");
        let i2 = r2.expect("second acquire must succeed");
        assert_ne!(i1, i2, "two acquires must get distinct indices");
    });
}

/// Model 2: Pool of 1. Thread A acquires, drops the guard (returns to
/// free list), thread B acquires. Proves the guard-Drop push to the
/// free list is visible to thread B's acquire.
#[test]
fn release_via_drop_then_reacquire() {
    loom::model(|| {
        let pool = Arc::new(SlabPool::<u64>::new(1, ImmediateReturn, NoOpLifecycle));

        // Acquire and immediately drop — guard Drop returns slab to free.
        {
            let _g = pool.try_acquire().unwrap();
            // Dropped here.
        }

        let p = pool.clone();
        let t = thread::spawn(move || {
            // Must succeed — the slab was returned by the Drop above.
            loop {
                if let Some(g) = p.try_acquire() {
                    return g.index();
                }
                loom::thread::yield_now();
            }
        });

        let idx = t.join().unwrap();
        assert_eq!(idx, 0, "only one slab — index must be 0");
    });
}

/// Model 3: Pool of 2. Two threads each acquire, use, and drop (release).
/// After both join, all 2 slabs must be back in the free list.
/// Proves the capacity invariant holds under concurrent lifecycle.
#[test]
fn concurrent_acquire_drop_restores_pool() {
    loom::model(|| {
        let pool = Arc::new(SlabPool::<u64>::new(2, ImmediateReturn, NoOpLifecycle));

        let p1 = pool.clone();
        let t1 = thread::spawn(move || {
            let mut g = p1.try_acquire().unwrap();
            *g = 42;
            // Guard dropped — slab returned to free.
        });

        let p2 = pool.clone();
        let t2 = thread::spawn(move || {
            let mut g = p2.try_acquire().unwrap();
            *g = 99;
            // Guard dropped — slab returned to free.
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // All slabs back in the free list.
        assert_eq!(pool.available(), 2, "all slabs must be returned after both threads join");
    });
}
