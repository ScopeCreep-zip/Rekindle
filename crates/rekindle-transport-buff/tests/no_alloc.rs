#![cfg(not(loom))]
//! Zero steady-state allocation proof.
//!
//! Proves that after construction, full submit→process→drain cycles through
//! ReorderRing, SlabPool, DispatchQueue, and CreditGuard perform zero heap
//! allocations. This is the mechanical proof that the per-item allocation
//! knee is closed.
//!
//! Uses a counting global allocator. All four primitives are tested in a
//! single `#[test]` function — one test in the binary means no parallel
//! test threads, no harness noise in the allocation counter. `cargo test`
//! runs this correctly with no special flags.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAlloc {
    allocs: AtomicUsize,
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocs.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc {
    allocs: AtomicUsize::new(0),
};

fn alloc_count() -> usize {
    ALLOC.allocs.load(Ordering::SeqCst)
}

#[inline(never)]
fn fence() {
    std::sync::atomic::fence(Ordering::SeqCst);
}

fn measure(f: impl FnOnce()) -> usize {
    fence();
    let before = alloc_count();
    fence();
    f();
    fence();
    let after = alloc_count();
    fence();
    after - before
}

#[test]
fn all_primitives_steady_state_zero_alloc() {
    use rekindle_transport_buff::pool::SlabPool;
    use rekindle_transport_buff::traits::{ImmediateReturn, NoOpLifecycle};
    use rekindle_transport_buff::{CreditGuard, DispatchQueue, ReorderRing, SpinWake};

    // --- ReorderRing ---
    let ring = ReorderRing::<u64>::new(64);
    for i in 0..64u64 {
        ring.publish(i, i).unwrap();
    }
    ring.drain_contiguous(|_, _| {});

    let delta = measure(|| {
        for cycle in 0..100u64 {
            let base = (cycle + 1) * 64;
            for i in 0..64u64 {
                let _ = ring.publish(base + i, base + i);
            }
            ring.drain_contiguous(|_, _| {});
        }
    });
    assert_eq!(delta, 0, "ReorderRing<u64> allocated {delta} times in steady state");

    // --- SlabPool ---
    let pool = SlabPool::<u64>::new(16, ImmediateReturn, NoOpLifecycle);
    let g = pool.try_acquire().unwrap();
    pool.release(g);
    pool.reclaim();

    let delta = measure(|| {
        for _ in 0..1000 {
            let g = pool.try_acquire().unwrap();
            pool.release(g);
            pool.reclaim();
        }
    });
    assert_eq!(delta, 0, "SlabPool<u64> allocated {delta} times in steady state");

    // --- DispatchQueue ---
    let q = DispatchQueue::<u64>::new(64, SpinWake);
    let _ = q.try_push(0, 0);
    let _ = q.pop();

    let delta = measure(|| {
        for i in 1..=1000u64 {
            let _ = q.try_push(i, i);
            let _ = q.pop();
        }
    });
    assert_eq!(delta, 0, "DispatchQueue<u64> allocated {delta} times in steady state");

    // --- CreditGuard ---
    let guard = CreditGuard::new(1_000_000);

    let delta = measure(|| {
        for _ in 0..10_000 {
            let _ = guard.try_reserve(100);
            guard.release(100);
        }
    });
    assert_eq!(delta, 0, "CreditGuard allocated {delta} times in steady state");
}
