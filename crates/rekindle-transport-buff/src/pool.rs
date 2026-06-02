//! [`SlabPool`] — closed-loop reusable-slab pool with return-gated reclaim.
//!
//! A fixed set of reusable slabs, allocated once. A consumer acquires a slab,
//! uses it, and releases it. Released slabs enter a pending gate — the pool
//! asks the consumer's [`ReturnGate`] impl "is this slab safe to reclaim?"
//! before returning it to the free list. This indirection keeps `io_uring`
//! (two-CQE `F_NOTIF` gate), RDMA completions, and any other transport
//! completion mechanism out of this crate.
//!
//! The free-list is a lock-free [`ArrayQueue<usize>`](crossbeam_queue::ArrayQueue)
//! from crossbeam-queue. No `Mutex`. No `Condvar`.
//!
//! # Allocation discipline
//!
//! [`try_acquire`](SlabPool::try_acquire) and [`release`](SlabPool::release)
//! perform zero heap allocations. [`reclaim`](SlabPool::reclaim) performs zero
//! heap allocations — it uses a two-pass pop-and-repush strategy over the
//! pending [`ArrayQueue`](crossbeam_queue::ArrayQueue) with no scratch buffer.
//!
//! # `!Send` preservation
//!
//! [`SlabGuard`] has `PhantomData<*const ()>` — it is **always** `!Send`,
//! regardless of `T`. If `T` is a `!Send` `io_uring` `FixedBuf` (`Rc`-based),
//! the guard is `!Send` too, and the compiler prevents it from crossing a
//! thread boundary. This is invariant 3 from the IPC spec, enforced by the
//! type system.
//!
//! # Replaces
//!
//! - `v3/bulk/pool.rs` — Mutex+Condvar pool
//! - `v3/pool/slab.rs` — redundant legacy pool

use core::fmt;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::panic::{RefUnwindSafe, UnwindSafe};

use crossbeam_queue::ArrayQueue;

use crate::loom_shim::cell::{MutPtr, UnsafeCell};
use crate::traits::{ImmediateReturn, NoOpLifecycle, ReturnGate, SlotLifecycle};

// ---------------------------------------------------------------------------
// SlabPool
// ---------------------------------------------------------------------------

/// A closed-loop pool of reusable slabs. Bounded by `capacity`. Allocates
/// once at construction. Zero per-operation allocation in the steady state.
///
/// `T` is the slab payload (e.g. the consumer's `FixedBuf` handle). The pool
/// is `T`-agnostic and does **not** require `T: Send`, so a `!Send` `T`
/// stays `!Send` via the [`SlabGuard`]'s `PhantomData<*const ()>`.
///
/// # Type parameters
///
/// - `G`: [`ReturnGate`] — decides when a released slab is safe to reclaim.
///   Defaults to [`ImmediateReturn`] (always immediately reclaimable).
/// - `L`: [`SlotLifecycle`] — construct-once / reset-on-reuse discipline.
///   Defaults to [`NoOpLifecycle`] (construct with `Default`, no-op reset).
pub struct SlabPool<T, G: ReturnGate = ImmediateReturn, L: SlotLifecycle<T> = NoOpLifecycle> {
    /// The slab storage. Each slot is initialized once at construction via
    /// `lifecycle.construct()`. Access is gated by ownership of the slab
    /// index (via `SlabGuard`).
    slabs: Box<[UnsafeCell<T>]>,

    /// Lock-free free-list of slab indices.
    /// `pop()` = acquire, `push()` = reclaim.
    free: ArrayQueue<usize>,

    /// Released slabs awaiting gate clearance. `(index, token)` pairs.
    /// Capacity == `slabs.len()`. A slab is in exactly one of
    /// {free, pending, in-use} at any time. If the invariant holds,
    /// `push()` to pending never fails.
    pending: ArrayQueue<(usize, G::Token)>,

    /// The return-gate policy. Consulted during `reclaim()`.
    gate: G,

    /// The slot lifecycle policy. `construct()` at init, `reset()` on reuse.
    lifecycle: L,

    /// Total slab count. Immutable after construction.
    capacity: usize,
}

// SAFETY: SlabPool is Send when T: Send, G: Send, L: Send.
// The ArrayQueue backing and UnsafeCell access are gated by index ownership.
unsafe impl<T: Send, G: ReturnGate, L: SlotLifecycle<T>> Send for SlabPool<T, G, L> {}

// SAFETY: SlabPool is Sync when T: Send, G: Sync, L: Sync.
// Multiple threads can call try_acquire/release/reclaim concurrently.
// G: Sync is required because reclaim() calls gate.is_clear() via &self.
// L: Sync is required because release() calls lifecycle.reset() via &self.
unsafe impl<T: Send, G: ReturnGate + Sync, L: SlotLifecycle<T> + Sync> Sync
    for SlabPool<T, G, L>
{
}

// Panic safety: if gate.is_clear() or lifecycle.reset() panics, items
// currently popped from pending are lost (their indices never return to
// free or pending). The pool's invariant is violated but no UB occurs.
// Document: gate.is_clear() and lifecycle.reset() must not panic.
impl<T, G: ReturnGate, L: SlotLifecycle<T>> UnwindSafe for SlabPool<T, G, L> {}
impl<T, G: ReturnGate, L: SlotLifecycle<T>> RefUnwindSafe for SlabPool<T, G, L> {}

impl<T, G: ReturnGate, L: SlotLifecycle<T>> SlabPool<T, G, L> {
    /// Create a pool of `capacity` reusable slabs.
    ///
    /// Each slab is initialized via `lifecycle.construct()` — this is the
    /// single allocation point. After construction, the steady state is
    /// allocation-free.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize, gate: G, lifecycle: L) -> Self {
        assert!(capacity > 0, "pool capacity must be non-zero");

        let slabs: Box<[UnsafeCell<T>]> = (0..capacity)
            .map(|_| UnsafeCell::new(lifecycle.construct()))
            .collect();

        let free = ArrayQueue::new(capacity);
        for i in 0..capacity {
            // Cannot fail: queue capacity == capacity, pushing exactly capacity items.
            free.push(i).unwrap_or_else(|_| {
                unreachable!("free-list push failed during init — capacity mismatch")
            });
        }

        // Pending capacity == slabs.len(). A slab is in exactly one of
        // {free, pending, in-use}. If the invariant holds, push to pending
        // never fails.
        let pending = ArrayQueue::new(capacity);

        Self {
            slabs,
            free,
            pending,
            gate,
            lifecycle,
            capacity,
        }
    }

    /// Non-blocking acquire. Returns `None` if the pool is exhausted —
    /// this is the closed-loop backpressure signal.
    ///
    /// The consumer's runtime decides how to wait (via [`WakeSink`](crate::WakeSink)).
    /// The pool does not spin, block, or park.
    #[inline]
    pub fn try_acquire(&self) -> Option<SlabGuard<'_, T, G, L>> {
        self.free.pop().map(|index| {
            // Acquire the RAII pointer guard at construction time.
            // Under loom, this calls UnsafeCell::get_mut() which returns
            // a MutPtr holding a Writing guard — loom tracks mutable
            // access for the entire SlabGuard lifetime. Under non-loom,
            // this is a zero-cost *mut T wrapper.
            //
            // SAFETY: we just popped this index from the free list.
            // Only one SlabGuard exists per index at a time (the free
            // list enforces this). We have exclusive access.
            let ptr = self.slabs[index].get_mut();
            SlabGuard {
                pool: self,
                index,
                ptr: ManuallyDrop::new(ptr),
                completed: false,
                _not_send: PhantomData,
            }
        })
    }

    /// Release a slab. The slab enters the pending gate — it is **not**
    /// immediately available for reacquire. Call [`reclaim`](Self::reclaim)
    /// when gate conditions change to move cleared slabs back to free.
    ///
    /// `lifecycle.reset()` is called on the slab before it enters pending.
    /// The `reset` call **must not allocate** — this is the contract that
    /// keeps the steady state allocation-free.
    pub fn release(&self, mut guard: SlabGuard<'_, T, G, L>) {
        let index = guard.index;

        // Explicitly drop the MutPtr to end loom's Writing tracking
        // BEFORE we call with_mut() below. Under loom, MutPtr holds a
        // Writing guard; if we mem::forget the whole SlabGuard, the
        // Writing guard leaks (is_writing stays true permanently), and
        // the subsequent with_mut() panics with "currently writing to cell."
        //
        // SAFETY: we are consuming the guard. The ptr will not be used
        // after this point — we access the slab via with_mut() instead.
        unsafe { ManuallyDrop::drop(&mut guard.ptr) };

        // Mark as completed so Drop doesn't push to free again.
        guard.completed = true;

        // Reset the slab (zeroize, clear, etc.)
        // SAFETY: we own the index — no other thread holds this slab.
        // The MutPtr was dropped above, so loom's is_writing is false.
        self.slabs[index].with_mut(|ptr| {
            // SAFETY: the slab is initialized (constructed in new(), through
            // at least one lifecycle). ptr is valid for &mut T.
            unsafe { self.lifecycle.reset(&mut *ptr) };
        });

        let token = self.gate.on_release(index);

        // Push to pending. Cannot fail if the {free, pending, in-use}
        // invariant holds: the slab was in-use, now goes to pending.
        self.pending.push((index, token)).unwrap_or_else(|_| {
            debug_assert!(
                false,
                "pending queue full — invariant violation: \
                 free.len() + pending.len() + in_use == capacity"
            );
        });
    }

    /// Move all gate-cleared slabs from pending back to the free list.
    ///
    /// The consumer calls this when it learns a gate condition changed
    /// (IPC: on each NOTIF CQE). Returns the count reclaimed.
    ///
    /// # Allocation
    ///
    /// Zero heap allocations. Uses a two-pass pop-and-repush strategy
    /// over the pending `ArrayQueue` with no scratch buffer. Items that
    /// are not yet clear are pushed back to pending immediately.
    ///
    /// # Concurrency
    ///
    /// Safe to call concurrently with `try_acquire` and `release`.
    /// Multiple concurrent `reclaim` calls are safe (`ArrayQueue` is MPMC)
    /// but wasteful — each call processes a snapshot of pending length,
    /// so concurrent calls may redundantly re-check the same items.
    ///
    /// # Ordering
    ///
    /// Reclaim order is **not FIFO**. The pending queue is an MPMC
    /// `ArrayQueue` — items may be popped and re-pushed in arbitrary
    /// order relative to their release time. A newly-released item can
    /// be checked before an older pending item. This is correct because
    /// `is_clear` is the reclaim gate, not arrival order. If ordered
    /// reclaim matters (e.g., processing NOTIFs in arrival order), the
    /// consumer must call `reclaim` when it knows all pending items are
    /// clear (e.g., after draining a batch of NOTIF CQEs), rather than
    /// relying on per-item FIFO ordering within `reclaim`.
    pub fn reclaim(&self) -> usize {
        let mut reclaimed: usize = 0;
        // Snapshot the pending length to bound iteration. Items added
        // by concurrent release() calls during this loop are processed
        // on the next reclaim() call — bounded iteration prevents an
        // infinite loop under concurrent load.
        let snapshot = self.pending.len();

        for _ in 0..snapshot {
            match self.pending.pop() {
                Some((index, token)) => {
                    if self.gate.is_clear(token) {
                        // Gate cleared — return to free list.
                        self.free.push(index).unwrap_or_else(|_| {
                            debug_assert!(
                                false,
                                "free-list push failed during reclaim — invariant violation"
                            );
                        });
                        reclaimed += 1;
                    } else {
                        // Not yet clear — push back to pending.
                        self.pending.push((index, token)).unwrap_or_else(|_| {
                            debug_assert!(
                                false,
                                "pending re-push failed during reclaim — invariant violation"
                            );
                        });
                    }
                }
                None => break,
            }
        }

        reclaimed
    }

    /// Total slab count. Immutable after construction.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of slabs currently in the free list (available for acquire).
    ///
    /// This is a point-in-time snapshot — it may be stale by the time the
    /// caller acts on it. Use `try_acquire()` as the scheduling primitive,
    /// not this method.
    #[inline]
    pub fn available(&self) -> usize {
        self.free.len()
    }

    /// Number of slabs currently in the pending gate (released but not
    /// yet reclaimable). Point-in-time snapshot.
    #[inline]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Approximate number of slabs currently in use (acquired but not released).
    ///
    /// **Non-atomic**: computed from two separate snapshots of `available()`
    /// and `pending_count()`. Under concurrent access, items can move between
    /// states between the two reads, so this value may transiently be
    /// inaccurate. Use `try_acquire()` for scheduling decisions, not this.
    #[inline]
    pub fn in_use(&self) -> usize {
        self.capacity
            .saturating_sub(self.available())
            .saturating_sub(self.pending_count())
    }

    /// Access the return gate.
    #[inline]
    pub fn gate(&self) -> &G {
        &self.gate
    }

    /// Access the slot lifecycle.
    #[inline]
    pub fn lifecycle(&self) -> &L {
        &self.lifecycle
    }
}

// No manual Drop impl needed. The slabs are `UnsafeCell<T>`, not
// `MaybeUninit<T>`. When `Box<[UnsafeCell<T>]>` drops, each UnsafeCell
// drops its inner `T` automatically. A manual `drop_in_place` here would
// double-drop: once from our manual call, once from the Box's automatic
// drop of UnsafeCell<T> → core::cell::UnsafeCell<T> → T::drop.
//
// If the slabs were `UnsafeCell<MaybeUninit<T>>` (where the compiler
// doesn't know whether the T is initialized), manual drop_in_place would
// be required. But they're `UnsafeCell<T>` — the T is always initialized
// (constructed in new()), and the compiler drops it.

impl<T, G: ReturnGate, L: SlotLifecycle<T>> fmt::Debug for SlabPool<T, G, L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlabPool")
            .field("capacity", &self.capacity)
            .field("available", &self.available())
            .field("pending", &self.pending_count())
            .field("in_use", &self.in_use())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// SlabGuard
// ---------------------------------------------------------------------------

/// RAII guard for an acquired slab. Provides [`Deref`]/[`DerefMut`] access
/// to the slab's contents. Must be passed to [`SlabPool::release`] to return
/// the slab through the gate.
///
/// # `!Send` — always, regardless of `T`
///
/// The guard has `PhantomData<*const ()>` — it is **unconditionally `!Send`**
/// and `!Sync`. Even if `T: Send`, the guard cannot cross thread boundaries.
/// This preserves the `io_uring` `FixedBuf` invariant: a buffer handle stays
/// on the thread that acquired it. The `SlabPool` itself is `Send + Sync`
/// (when `T: Send`), so it can be shared — but each guard is thread-local.
///
/// # Drop behavior
///
/// If dropped without calling [`SlabPool::release`] (e.g., on panic or
/// early return), the guard calls `lifecycle.reset()` on the slab and
/// returns it directly to the free list, **bypassing the gate**. This
/// means the slab is available for immediate reacquire — which is safe
/// for non-gated pools (`ImmediateReturn`) but may violate the gate
/// invariant for transport-gated pools. Transport consumers must ensure
/// guards are always released via `SlabPool::release`, not dropped.
///
/// # `!Send` proof (`compile_fail`)
///
/// A `SlabGuard` is unconditionally `!Send`, even when `T: Send`.
/// This is verified by asserting the `Send` bound fails at the type level:
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<rekindle_transport_buff::SlabGuard<'_, u64>>();
/// ```
pub struct SlabGuard<'pool, T, G: ReturnGate = ImmediateReturn, L: SlotLifecycle<T> = NoOpLifecycle>
{
    pool: &'pool SlabPool<T, G, L>,
    index: usize,
    /// RAII pointer guard that keeps loom's causality tracking alive for
    /// the entire lifetime of this `SlabGuard`. Under loom, this is
    /// `loom::cell::MutPtr<T>` whose `_guard: Writing` field tracks
    /// mutable access. Under non-loom, this is a thin `*mut T` wrapper
    /// with no runtime cost.
    ///
    /// This is the thingbuf `Ref` pattern: the `Ref` struct holds
    /// `ptr: MutPtr<MaybeUninit<T>>` so that `Deref`/`DerefMut` can
    /// dereference through the held guard without escaping a closure
    /// scope. Without this, returning `&T` from `UnsafeCell::with()`
    /// escapes loom's tracking — the `Reading` guard drops when `with`
    /// returns, but the `&T` lives on.
    /// Wrapped in `ManuallyDrop` so that `release()` can explicitly drop
    /// the `MutPtr` (ending loom's `Writing` tracking) before calling
    /// `with_mut()` on the same cell. Without `ManuallyDrop`, `release()`
    /// would have to use `mem::forget(guard)` which leaks the `MutPtr`'s
    /// `Writing` guard — under loom, this leaves `is_writing == true`
    /// permanently, causing the subsequent `with_mut()` to panic with
    /// "currently writing to cell."
    ptr: ManuallyDrop<MutPtr<T>>,
    /// Set to `true` by `release()` after explicitly dropping the `MutPtr`.
    /// When `true`, `Drop` is a no-op (the slab was already handled by
    /// `release()`). When `false` (the normal drop path), `Drop` resets
    /// the slab and returns it to the free list.
    completed: bool,
    /// `*const ()` is `!Send` and `!Sync` — the guard is unconditionally
    /// `!Send` regardless of `T`.
    _not_send: PhantomData<*const ()>,
}

impl<T, G: ReturnGate, L: SlotLifecycle<T>> SlabGuard<'_, T, G, L> {
    /// The slab's index in the pool. Useful for `io_uring` `buf_index` or
    /// RDMA work-request correlation.
    #[inline]
    pub fn index(&self) -> usize {
        self.index
    }
}

impl<T, G: ReturnGate, L: SlotLifecycle<T>> Deref for SlabGuard<'_, T, G, L> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: we hold the index — exclusive access. The slab was
        // initialized in new() via construct(). No concurrent access
        // is possible because the index was popped from the free-list
        // (only one SlabGuard exists per index at a time).
        //
        // We dereference through the held MutPtr guard, NOT through a
        // with() closure. The MutPtr's RAII guard (under loom: the
        // Writing field) keeps loom's causality tracking alive for our
        // entire lifetime. Returning &T from with() would escape the
        // closure scope and drop the tracking guard prematurely.
        //
        // This is the thingbuf Ref::deref pattern (thingbuf lib.rs:543-549):
        //   unsafe { &*self.ptr.deref().as_ptr() }
        // Adapted for our case where the UnsafeCell holds T directly
        // (not MaybeUninit<T>), so we deref to &T, not &MaybeUninit<T>.
        // ManuallyDrop<MutPtr<T>>::deref() returns &MutPtr<T>.
        // We need MutPtr<T>::deref() which returns &mut T.
        // Deref through ManuallyDrop first (*self.ptr → MutPtr<T>),
        // then call MutPtr::deref() → &mut T, then reborrow as &T.
        unsafe { &*(*self.ptr).deref() }
    }
}

impl<T, G: ReturnGate, L: SlotLifecycle<T>> DerefMut for SlabGuard<'_, T, G, L> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: same as Deref — exclusive access via index ownership.
        // &mut self guarantees no other &/&mut SlabGuard reference exists.
        // The MutPtr guard keeps loom tracking alive.
        unsafe { (*self.ptr).deref() }
    }
}

impl<T, G: ReturnGate, L: SlotLifecycle<T>> Drop for SlabGuard<'_, T, G, L> {
    fn drop(&mut self) {
        if self.completed {
            // release() already handled the slab — ptr was explicitly
            // dropped, slab was reset, and pushed to pending. Nothing to do.
            return;
        }

        // Guard dropped without release() — reset the slab and return
        // directly to the free list (bypassing the gate).
        //
        // lifecycle.reset() is called so the next acquirer does not get
        // stale/unzeroized contents — this is the security fix for zeroize
        // lifecycles on the panic path.
        //
        // NOTE: For transport-gated pools (io_uring NOTIF), bypassing the
        // gate on drop means the buffer may be reacquired before the kernel
        // is done with it. Transport consumers MUST ensure guards go through
        // release(), not drop. The drop path is a last-resort safety net.
        //
        // We call reset through the held MutPtr guard (which we still own
        // in drop — Drop takes &mut self, so self.ptr is still live).
        // SAFETY: slab is initialized, we have exclusive access via index.
        unsafe { self.pool.lifecycle.reset((*self.ptr).deref()) };

        // Drop the MutPtr to end loom's Writing tracking before pushing
        // the index back to the free list.
        // SAFETY: ptr will not be used after this.
        unsafe { ManuallyDrop::drop(&mut self.ptr) };

        self.pool.free.push(self.index).unwrap_or_else(|_| {
            debug_assert!(
                false,
                "free-list push failed in SlabGuard::drop — invariant violation"
            );
        });
    }
}

impl<T, G: ReturnGate, L: SlotLifecycle<T>> fmt::Debug for SlabGuard<'_, T, G, L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlabGuard")
            .field("index", &self.index)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use crate::traits::{ImmediateReturn, NoOpLifecycle};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    #[test]
    fn new_initializes_all_free() {
        let pool = SlabPool::<u64>::new(8, ImmediateReturn, NoOpLifecycle);
        assert_eq!(pool.capacity(), 8);
        assert_eq!(pool.available(), 8);
        assert_eq!(pool.pending_count(), 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    #[should_panic(expected = "non-zero")]
    fn new_rejects_zero() {
        let _pool = SlabPool::<u64>::new(0, ImmediateReturn, NoOpLifecycle);
    }

    #[test]
    fn acquire_and_release_cycle() {
        let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);

        let guard = pool.try_acquire().unwrap();
        assert_eq!(pool.available(), 3);
        assert_eq!(pool.in_use(), 1);

        pool.release(guard);
        assert_eq!(pool.pending_count(), 1);

        // ImmediateReturn: reclaim returns it immediately.
        let reclaimed = pool.reclaim();
        assert_eq!(reclaimed, 1);
        assert_eq!(pool.available(), 4);
    }

    #[test]
    fn exhaustion_returns_none() {
        let pool = SlabPool::<u64>::new(2, ImmediateReturn, NoOpLifecycle);
        let _g1 = pool.try_acquire().unwrap();
        let _g2 = pool.try_acquire().unwrap();
        assert!(pool.try_acquire().is_none());
        assert_eq!(pool.in_use(), 2);
    }

    #[test]
    fn deref_access() {
        let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);
        let mut guard = pool.try_acquire().unwrap();
        *guard = 42;
        assert_eq!(*guard, 42);
    }

    #[test]
    fn guard_drop_resets_and_returns_to_free() {
        static RESET_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct ResetCounter;
        impl SlotLifecycle<u64> for ResetCounter {
            fn construct(&self) -> u64 {
                0
            }
            fn reset(&self, _slot: &mut u64) {
                RESET_COUNT.fetch_add(1, Relaxed);
            }
        }

        RESET_COUNT.store(0, Relaxed);
        let pool = SlabPool::new(4, ImmediateReturn, ResetCounter);
        {
            let mut guard = pool.try_acquire().unwrap();
            *guard = 42;
            assert_eq!(pool.available(), 3);
            // Guard dropped without release — should reset AND return to free.
        }
        assert_eq!(pool.available(), 4);
        assert_eq!(pool.pending_count(), 0);
        assert_eq!(RESET_COUNT.load(Relaxed), 1, "reset must be called on drop");
    }

    #[test]
    fn lifecycle_construct_called() {
        static CONSTRUCT_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct CountingLifecycle;
        impl SlotLifecycle<u64> for CountingLifecycle {
            fn construct(&self) -> u64 {
                CONSTRUCT_COUNT.fetch_add(1, Relaxed);
                0
            }
            fn reset(&self, _slot: &mut u64) {}
        }

        CONSTRUCT_COUNT.store(0, Relaxed);
        let _pool = SlabPool::new(5, ImmediateReturn, CountingLifecycle);
        assert_eq!(CONSTRUCT_COUNT.load(Relaxed), 5);
    }

    #[test]
    fn lifecycle_reset_called_on_release() {
        static RESET_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct CountingLifecycle;
        impl SlotLifecycle<u64> for CountingLifecycle {
            fn construct(&self) -> u64 {
                0
            }
            fn reset(&self, _slot: &mut u64) {
                RESET_COUNT.fetch_add(1, Relaxed);
            }
        }

        RESET_COUNT.store(0, Relaxed);
        let pool = SlabPool::new(4, ImmediateReturn, CountingLifecycle);
        let guard = pool.try_acquire().unwrap();
        pool.release(guard);
        assert_eq!(RESET_COUNT.load(Relaxed), 1);
    }

    /// Mock gate that clears only when `clear_index` is called.
    struct MockGate {
        cleared: std::sync::Mutex<std::collections::HashSet<usize>>,
    }

    impl MockGate {
        fn new() -> Self {
            Self {
                cleared: std::sync::Mutex::new(std::collections::HashSet::new()),
            }
        }
        fn clear_index(&self, index: usize) {
            self.cleared.lock().unwrap().insert(index);
        }
    }

    impl ReturnGate for MockGate {
        type Token = usize;

        fn is_clear(&self, token: usize) -> bool {
            self.cleared.lock().unwrap().contains(&token)
        }

        fn on_release(&self, index: usize) -> usize {
            index
        }
    }

    #[test]
    fn gated_release_delays_reclaim() {
        let pool: SlabPool<u64, MockGate> = SlabPool::new(4, MockGate::new(), NoOpLifecycle);
        let guard = pool.try_acquire().unwrap();
        let idx = guard.index();
        pool.release(guard);

        // Not cleared yet — reclaim returns 0.
        assert_eq!(pool.reclaim(), 0);
        assert_eq!(pool.available(), 3);
        assert_eq!(pool.pending_count(), 1);

        // Clear the gate for this index.
        pool.gate().clear_index(idx);
        assert_eq!(pool.reclaim(), 1);
        assert_eq!(pool.available(), 4);
        assert_eq!(pool.pending_count(), 0);
    }

    #[test]
    fn full_cycle_all_slabs() {
        let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);

        let guards: Vec<_> = (0..4).map(|_| pool.try_acquire().unwrap()).collect();
        assert!(pool.try_acquire().is_none());

        for g in guards {
            pool.release(g);
        }
        assert_eq!(pool.pending_count(), 4);
        assert_eq!(pool.available(), 0);

        assert_eq!(pool.reclaim(), 4);
        assert_eq!(pool.available(), 4);
        assert_eq!(pool.pending_count(), 0);
    }

    #[test]
    fn reclaim_no_allocation() {
        // Verify reclaim does not allocate by running it many times.
        // The no_alloc.rs integration test uses a counting allocator for
        // the full proof; this test just confirms the code path works
        // without panicking.
        let pool = SlabPool::<u64>::new(8, ImmediateReturn, NoOpLifecycle);
        for _ in 0..100 {
            let guard = pool.try_acquire().unwrap();
            pool.release(guard);
            pool.reclaim();
        }
        assert_eq!(pool.available(), 8);
    }

    #[test]
    fn in_use_is_approximate() {
        let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);
        assert_eq!(pool.in_use(), 0);
        let _g1 = pool.try_acquire().unwrap();
        // in_use is approximate but should be ~1 with no concurrency.
        assert_eq!(pool.in_use(), 1);
    }

    #[test]
    fn pool_debug_format() {
        let pool = SlabPool::<u64>::new(8, ImmediateReturn, NoOpLifecycle);
        let debug = format!("{pool:?}");
        assert!(debug.contains("SlabPool"));
        assert!(debug.contains("capacity: 8"));
        assert!(debug.contains("available: 8"));
    }

    #[test]
    fn guard_index_is_valid() {
        let pool = SlabPool::<u64>::new(4, ImmediateReturn, NoOpLifecycle);
        let guard = pool.try_acquire().unwrap();
        assert!(guard.index() < 4);
    }
}
