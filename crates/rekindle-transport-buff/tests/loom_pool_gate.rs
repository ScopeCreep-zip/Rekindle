//! Loom model-checking tests for [`SlabPool`] with a non-trivial [`ReturnGate`].
//!
//! Run with `RUSTFLAGS="--cfg loom" cargo test --test loom_pool_gate`.
//!
//! These tests exercise the **deferred-reclaim race**: a `ReturnGate` that
//! starts unclear and is cleared asynchronously from a separate thread,
//! racing against `reclaim()` and `try_acquire()`. This is the production
//! io_uring `F_NOTIF` path.
//!
//! All gate atomics use `loom::sync::atomic` so loom's DPOR can track the
//! Release/Acquire interaction between `clear_index` and `is_clear`.

#![cfg(loom)]

use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

use rekindle_transport_buff::pool::SlabPool;
use rekindle_transport_buff::traits::{NoOpLifecycle, ReturnGate, SlotLifecycle};

// ---------------------------------------------------------------------------
// AsyncGate — a ReturnGate backed by loom-tracked atomics
// ---------------------------------------------------------------------------

struct AsyncGate {
    flags: Arc<Vec<AtomicBool>>,
}

impl AsyncGate {
    fn new(capacity: usize) -> Self {
        let flags: Vec<AtomicBool> = (0..capacity)
            .map(|_| AtomicBool::new(false))
            .collect();
        Self {
            flags: Arc::new(flags),
        }
    }

    fn clear_index(&self, index: usize) {
        self.flags[index].store(true, Ordering::Release);
    }
}

impl ReturnGate for AsyncGate {
    type Token = usize;

    fn is_clear(&self, token: usize) -> bool {
        self.flags[token].load(Ordering::Acquire)
    }

    fn on_release(&self, index: usize) -> usize {
        index
    }
}

struct LoomCountLifecycle {
    resets: Arc<AtomicUsize>,
}

impl SlotLifecycle<u64> for LoomCountLifecycle {
    fn construct(&self) -> u64 {
        0
    }
    fn reset(&self, _slot: &mut u64) {
        self.resets.fetch_add(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Models — all use Builder::preemption_bound to keep state space tractable
// ---------------------------------------------------------------------------

/// Model 1: Release on main, clear from spawned thread, reclaim on main.
#[test]
fn async_gate_release_clear_reclaim() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(2);
    builder.check(|| {
        let resets = Arc::new(AtomicUsize::new(0));
        let gate = AsyncGate::new(1);
        let lifecycle = LoomCountLifecycle {
            resets: resets.clone(),
        };

        let pool = Arc::new(SlabPool::new(1, gate, lifecycle));

        let guard = pool.try_acquire().expect("pool has 1 slab");
        let idx = guard.index();

        pool.release(guard);

        let p = pool.clone();
        let clearer = thread::spawn(move || {
            p.gate().clear_index(idx);
        });

        let reclaimed = pool.reclaim();
        clearer.join().unwrap();

        if reclaimed == 0 {
            let second = pool.reclaim();
            assert_eq!(second, 1, "second reclaim must succeed after clearer joined");
        }

        assert_eq!(pool.available(), 1);
        assert_eq!(pool.pending_count(), 0);
        assert_eq!(resets.load(Ordering::Relaxed), 1);
    });
}

/// Model 2: Two threads each acquire+release, cross-clear each other's gate.
#[test]
fn async_gate_concurrent_release_and_reclaim() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(2);
    builder.check(|| {
        let gate = AsyncGate::new(2);
        let pool = Arc::new(SlabPool::<u64, AsyncGate>::new(2, gate, NoOpLifecycle));

        let pa = pool.clone();
        let ta = thread::spawn(move || {
            let g0 = pa.try_acquire().unwrap();
            let idx0 = g0.index();
            pa.release(g0);
            pa.gate().clear_index(idx0 ^ 1);
        });

        let pb = pool.clone();
        let tb = thread::spawn(move || {
            let g1 = pb.try_acquire().unwrap();
            let idx1 = g1.index();
            pb.release(g1);
            pb.gate().clear_index(idx1 ^ 1);
        });

        ta.join().unwrap();
        tb.join().unwrap();

        let mut total = 0;
        total += pool.reclaim();
        total += pool.reclaim();
        assert_eq!(total, 2, "both slabs must be reclaimed");
        assert_eq!(pool.available(), 2);
    });
}

/// Model 3: Release + clear, then reclaim + acquire on another thread.
#[test]
fn async_gate_acquire_after_reclaim() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(2);
    builder.check(|| {
        let gate = AsyncGate::new(1);
        let pool = Arc::new(SlabPool::<u64, AsyncGate>::new(1, gate, NoOpLifecycle));

        let guard = pool.try_acquire().unwrap();
        let idx = guard.index();
        pool.release(guard);
        pool.gate().clear_index(idx);

        let p = pool.clone();
        let t = thread::spawn(move || {
            loop {
                if p.reclaim() > 0 {
                    break;
                }
                loom::thread::yield_now();
            }
            let g = p.try_acquire().expect("slab must be available after reclaim");
            g.index()
        });

        let acquired_idx = t.join().unwrap();
        assert_eq!(acquired_idx, idx);
    });
}
