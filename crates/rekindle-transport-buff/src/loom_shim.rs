//! Conditional re-exports for loom model-checking.
//!
//! Under `cfg(loom)`, all atomics and `UnsafeCell` are loom-instrumented — every
//! operation is a model-checker branch point. Under `cfg(not(loom))`, the types
//! are zero-cost re-exports of `core`/`std`.
//!
//! The `UnsafeCell` wrapper on the non-loom side provides the `with`/`with_mut`
//! closure API that matches loom's causality-tracking API. All source files use
//! `crate::loom_shim::cell::UnsafeCell` — never `core::cell::UnsafeCell` directly.
//!
//! Pattern source: `crossbeam-utils/src/lib.rs` `cfg(crossbeam_loom)` gating.
//!
//! # Convention
//!
//! **Both** the `cfg(loom)` and `cfg(not(loom))` sides export **only what
//! src/ files actually import**. Adding an export without a consumer in src/
//! is a `deny(unused_imports)` error under either cfg.
//!
//! Test files in `tests/` are separate compilation units. Under `cfg(loom)`
//! they import from `loom::` directly; under `cfg(not(loom))` from `std::`.
//! They do **not** go through this shim.

// ---------------------------------------------------------------------------
// cfg(loom) — loom-instrumented types
// ---------------------------------------------------------------------------

#[cfg(loom)]
pub(crate) mod sync {
    pub(crate) mod atomic {
        // Only what src/ files import:
        //   reorder.rs: AtomicUsize, Ordering
        //   credit.rs:  AtomicU64, Ordering
        pub(crate) use loom::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    }
}

#[cfg(loom)]
pub(crate) mod cell {
    pub(crate) use loom::cell::{MutPtr, UnsafeCell};
}

// ---------------------------------------------------------------------------
// cfg(not(loom)) — std types with loom-compatible UnsafeCell wrapper
// ---------------------------------------------------------------------------

#[cfg(not(loom))]
pub(crate) mod sync {
    pub(crate) mod atomic {
        // Only what src/ files import:
        //   reorder.rs: AtomicUsize, Ordering
        //   credit.rs:  AtomicU64, Ordering
        pub(crate) use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    }
}

#[cfg(not(loom))]
pub(crate) mod cell {
    /// `UnsafeCell` wrapper providing loom's `with`/`with_mut` closure API
    /// AND loom's `get`/`get_mut` RAII guard API.
    ///
    /// Under loom, `loom::cell::UnsafeCell` tracks causality: every access
    /// through `with`/`with_mut` is a model-checker branch point that detects
    /// data races. The `get()`/`get_mut()` methods return `ConstPtr`/`MutPtr`
    /// RAII guards whose `_guard` field keeps loom's causality tracking alive
    /// for the guard's entire lifetime — not just the closure body.
    ///
    /// This wrapper provides the same API over `core::cell::UnsafeCell`
    /// so all source code uses one API regardless of cfg.
    ///
    /// # The closure vs. guard distinction
    ///
    /// `with`/`with_mut`: tracking ends when the closure returns. A pointer
    /// or reference returned from the closure escapes tracking. Use these
    /// for short-lived accesses (`ptr::read`, `ptr::write`) that do not outlive
    /// the closure.
    ///
    /// `get`/`get_mut`: tracking lives as long as the returned `ConstPtr`/
    /// `MutPtr` guard. Use these when the access must outlive a single
    /// function call — e.g., `SlabGuard::Deref` returning `&T` that lives
    /// as long as the guard. This is the thingbuf `Ref` pattern.
    #[derive(Debug)]
    pub(crate) struct UnsafeCell<T>(core::cell::UnsafeCell<T>);

    impl<T> UnsafeCell<T> {
        #[inline]
        pub(crate) fn new(data: T) -> Self {
            Self(core::cell::UnsafeCell::new(data))
        }

        /// Immutable access bounded by the closure. The pointer must not
        /// escape the closure — if it does, the access outlives tracking.
        #[inline]
        pub(crate) fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
            f(self.0.get().cast_const())
        }

        /// Mutable access bounded by the closure. Caller must ensure
        /// exclusive access. The pointer must not escape the closure.
        #[inline]
        pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
            f(self.0.get())
        }

        /// Get a mutable RAII guard. The access is tracked for the
        /// guard's entire lifetime, not just a closure scope. Under loom,
        /// this returns `loom::cell::MutPtr<T>` whose `_guard: Writing`
        /// keeps causality tracking alive for the guard's entire lifetime.
        ///
        /// Use this when the access must outlive a single function call —
        /// e.g., `SlabGuard::Deref` returning `&T` that lives as long as
        /// the guard. This is the thingbuf `Ref` pattern.
        #[inline]
        pub(crate) fn get_mut(&self) -> MutPtr<T> {
            MutPtr(self.0.get())
        }
    }

    // SAFETY: UnsafeCell<T> is Send if T: Send — same as core::cell::UnsafeCell.
    // We add no shared state beyond what core::cell::UnsafeCell provides.
    // The Send/Sync bounds on the outer type (ReorderRing, SlabPool) are what
    // actually govern thread safety.
    unsafe impl<T: Send> Send for UnsafeCell<T> {}
    // SAFETY: Sync if T: Send — access is gated by the outer type's
    // synchronization (per-slot atomics in ReorderRing, index ownership
    // in SlabPool). The UnsafeCell itself adds no shared mutable state.
    unsafe impl<T: Send> Sync for UnsafeCell<T> {}

    /// Mutable RAII pointer guard. Non-loom equivalent of
    /// `loom::cell::MutPtr<T>`. Under non-loom, this is a thin wrapper
    /// around `*mut T` with no runtime cost. Under loom, loom's own
    /// `MutPtr` holds `_guard: rt::cell::Writing` that keeps causality
    /// tracking alive for the guard's entire lifetime.
    ///
    /// The thingbuf crate's `MutPtr` (thingbuf `loom.rs` lines 260-279)
    /// is this exact pattern.
    #[derive(Debug)]
    pub(crate) struct MutPtr<T: ?Sized>(*mut T);

    impl<T: ?Sized> MutPtr<T> {
        /// Dereference the pointer.
        ///
        /// # Safety
        ///
        /// The caller must ensure the pointer is valid and that exclusive
        /// access is held.
        #[inline]
        #[allow(clippy::mut_from_ref)]
        pub(crate) unsafe fn deref(&self) -> &mut T {
            // SAFETY: caller guarantees the pointer is valid and exclusive
            // access is held (enforced by SlabGuard's index ownership).
            unsafe { &mut *self.0 }
        }

    }
}
