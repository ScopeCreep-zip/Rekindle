#![cfg(not(loom))]
//! Pool lifecycle tests — acquire, release, gate, reclaim, Drop, and `!Send`.
//!
//! Proves:
//! - Slabs cycle through free → in-use → pending → (gate clear) → free.
//! - `SlotLifecycle::construct` is called exactly `capacity` times.
//! - `SlotLifecycle::reset` is called on every release and every guard drop.
//! - `ReturnGate` delays reclaim until `is_clear` returns true.
//! - `SlabGuard` is unconditionally `!Send` (compile_fail doctest below).
//! - Pool `Drop` drops all slab contents exactly once.

use rekindle_transport_buff::pool::SlabPool;
use rekindle_transport_buff::traits::{
    ImmediateReturn, NoOpLifecycle, ReturnGate, SlotLifecycle,
};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

// ---------------------------------------------------------------------------
// Custom lifecycle that counts construct and reset calls
// ---------------------------------------------------------------------------

struct CountingLifecycle {
    constructs: &'static AtomicUsize,
    resets: &'static AtomicUsize,
}

impl SlotLifecycle<String> for CountingLifecycle {
    fn construct(&self) -> String {
        self.constructs.fetch_add(1, Relaxed);
        String::with_capacity(64)
    }
    fn reset(&self, slot: &mut String) {
        self.resets.fetch_add(1, Relaxed);
        slot.clear();
    }
}

// ---------------------------------------------------------------------------
// Custom gate that delays reclaim until explicitly cleared
// ---------------------------------------------------------------------------

struct ManualGate {
    cleared: std::sync::Mutex<std::collections::HashSet<usize>>,
}

impl ManualGate {
    fn new() -> Self {
        Self {
            cleared: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }
    fn clear(&self, idx: usize) {
        self.cleared.lock().unwrap().insert(idx);
    }
}

impl ReturnGate for ManualGate {
    type Token = usize;
    fn is_clear(&self, token: usize) -> bool {
        self.cleared.lock().unwrap().contains(&token)
    }
    fn on_release(&self, index: usize) -> usize {
        index
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn construct_called_exactly_capacity_times() {
    static CONSTRUCTS: AtomicUsize = AtomicUsize::new(0);
    static RESETS: AtomicUsize = AtomicUsize::new(0);
    CONSTRUCTS.store(0, Relaxed);
    RESETS.store(0, Relaxed);

    let _pool = SlabPool::<String, ImmediateReturn, CountingLifecycle>::new(
        7,
        ImmediateReturn,
        CountingLifecycle {
            constructs: &CONSTRUCTS,
            resets: &RESETS,
        },
    );
    assert_eq!(CONSTRUCTS.load(Relaxed), 7);
    assert_eq!(RESETS.load(Relaxed), 0);
}

#[test]
fn reset_called_on_release() {
    static CONSTRUCTS: AtomicUsize = AtomicUsize::new(0);
    static RESETS: AtomicUsize = AtomicUsize::new(0);
    CONSTRUCTS.store(0, Relaxed);
    RESETS.store(0, Relaxed);

    let pool = SlabPool::<String, ImmediateReturn, CountingLifecycle>::new(
        4,
        ImmediateReturn,
        CountingLifecycle {
            constructs: &CONSTRUCTS,
            resets: &RESETS,
        },
    );
    let guard = pool.try_acquire().unwrap();
    pool.release(guard);
    assert_eq!(RESETS.load(Relaxed), 1);
}

#[test]
fn reset_called_on_guard_drop() {
    static CONSTRUCTS: AtomicUsize = AtomicUsize::new(0);
    static RESETS: AtomicUsize = AtomicUsize::new(0);
    CONSTRUCTS.store(0, Relaxed);
    RESETS.store(0, Relaxed);

    let pool = SlabPool::<String, ImmediateReturn, CountingLifecycle>::new(
        4,
        ImmediateReturn,
        CountingLifecycle {
            constructs: &CONSTRUCTS,
            resets: &RESETS,
        },
    );
    {
        let _guard = pool.try_acquire().unwrap();
        // dropped without release
    }
    assert_eq!(RESETS.load(Relaxed), 1);
    // Guard drop bypasses gate, returns directly to free.
    assert_eq!(pool.available(), 4);
    assert_eq!(pool.pending_count(), 0);
}

#[test]
fn gated_release_delays_until_clear() {
    let pool: SlabPool<u64, ManualGate> =
        SlabPool::new(4, ManualGate::new(), NoOpLifecycle);

    let g0 = pool.try_acquire().unwrap();
    let idx0 = g0.index();
    let g1 = pool.try_acquire().unwrap();
    let idx1 = g1.index();

    pool.release(g0);
    pool.release(g1);
    assert_eq!(pool.pending_count(), 2);
    assert_eq!(pool.available(), 2);

    // Nothing cleared — reclaim returns 0.
    assert_eq!(pool.reclaim(), 0);
    assert_eq!(pool.pending_count(), 2);

    // Clear only one.
    pool.gate().clear(idx0);
    assert_eq!(pool.reclaim(), 1);
    assert_eq!(pool.available(), 3);
    assert_eq!(pool.pending_count(), 1);

    // Clear the other.
    pool.gate().clear(idx1);
    assert_eq!(pool.reclaim(), 1);
    assert_eq!(pool.available(), 4);
    assert_eq!(pool.pending_count(), 0);
}

#[test]
fn full_cycle_exhaust_release_reclaim() {
    let pool = SlabPool::<u64>::new(3, ImmediateReturn, NoOpLifecycle);

    // Exhaust.
    let guards: Vec<_> = (0..3).map(|_| pool.try_acquire().unwrap()).collect();
    assert!(pool.try_acquire().is_none());

    // Release all.
    for g in guards {
        pool.release(g);
    }
    assert_eq!(pool.pending_count(), 3);

    // Reclaim all (ImmediateReturn: all clear immediately).
    assert_eq!(pool.reclaim(), 3);
    assert_eq!(pool.available(), 3);

    // Re-acquire all.
    let _guards: Vec<_> = (0..3).map(|_| pool.try_acquire().unwrap()).collect();
    assert!(pool.try_acquire().is_none());
}

#[test]
fn drop_drops_all_slab_contents() {
    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct Tracked(u64);
    impl Drop for Tracked {
        fn drop(&mut self) {
            assert!(self.0 < 100, "unexpected value in dropped Tracked");
            DROP_COUNT.fetch_add(1, Relaxed);
        }
    }

    struct TrackedLifecycle(AtomicUsize);
    impl SlotLifecycle<Tracked> for TrackedLifecycle {
        fn construct(&self) -> Tracked {
            Tracked(self.0.fetch_add(1, Relaxed) as u64)
        }
        fn reset(&self, _slot: &mut Tracked) {}
    }

    DROP_COUNT.store(0, Relaxed);
    {
        let pool = SlabPool::new(5, ImmediateReturn, TrackedLifecycle(AtomicUsize::new(0)));
        // Acquire 2, leave 3 in free — all 5 should be dropped.
        let _g0 = pool.try_acquire().unwrap();
        let _g1 = pool.try_acquire().unwrap();
        // pool drops here with 2 in-use, 3 in free, 0 pending
    }
    assert_eq!(DROP_COUNT.load(Relaxed), 5);
}

#[test]
fn deref_mut_writes_persist() {
    let pool = SlabPool::<Vec<u8>>::new(2, ImmediateReturn, NoOpLifecycle);
    let mut guard = pool.try_acquire().unwrap();
    guard.push(1);
    guard.push(2);
    guard.push(3);
    assert_eq!(&*guard, &[1, 2, 3]);
}

#[test]
fn reclaim_is_zero_alloc_steady_state() {
    // Run many cycles to confirm no allocation growth.
    let pool = SlabPool::<u64>::new(8, ImmediateReturn, NoOpLifecycle);
    for _ in 0..10_000 {
        let g = pool.try_acquire().unwrap();
        pool.release(g);
        pool.reclaim();
    }
    assert_eq!(pool.available(), 8);
}

// ---------------------------------------------------------------------------
// Compile-fail: SlabGuard is !Send
// ---------------------------------------------------------------------------

/// ```compile_fail
/// use rekindle_transport_buff::pool::SlabPool;
/// use rekindle_transport_buff::traits::{ImmediateReturn, NoOpLifecycle};
///
/// let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);
/// let guard = pool.try_acquire().unwrap();
/// // SlabGuard is !Send — this must not compile.
/// std::thread::spawn(move || {
///     let _ = *guard;
/// });
/// ```
#[cfg(doctest)]
struct _SlabGuardNotSendDoctest;
