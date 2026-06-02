//! [`CreditGuard`] — open-loop atomic CAS credit counter with ceiling.
//!
//! The recv side cannot make the peer wait — it sheds load by refusing
//! admission past the ceiling. This is the replacement for the
//! `GlobalMemoryGuard` in the IPC crate's recv path.
//!
//! # Why a counter, not a pool
//!
//! The send side is demand-driven (the local application can be made to
//! wait → closed-loop [`SlabPool`](crate::SlabPool)). The recv side is
//! peer-driven (arrivals cannot be refused in time → open-loop credit
//! counter that sheds). A recv pool would deadlock under sustained
//! peer-driven load.
//!
//! # Ordering
//!
//! - [`try_reserve`](CreditGuard::try_reserve): `AcqRel` on the successful
//!   CAS so the reservation happens-before the caller's use of the admitted
//!   item. `Relaxed` on failure — the CAS returns the fresh actual value
//!   regardless of ordering, matching the crossbeam `ArrayQueue::push`
//!   and `ArrayQueue::pop` pattern (`SeqCst`/`Relaxed` there; `AcqRel`/
//!   `Relaxed` here because a single counter needs no total order).
//!   [`Backoff::spin()`](crossbeam_utils::Backoff::spin) on CAS failure
//!   to reduce cache-line contention under high thread counts.
//! - [`release`](CreditGuard::release): `Release` so the freed credit is
//!   visible to the next reserver's `Acquire` load.
//! - [`inflight`](CreditGuard::inflight): `Relaxed` — observability only,
//!   not a synchronization point.

use core::fmt;
use core::panic::{RefUnwindSafe, UnwindSafe};

use crossbeam_utils::{Backoff, CachePadded};

use crate::loom_shim::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// CreditGuard
// ---------------------------------------------------------------------------

/// Open-loop admission bound. A CAS-loop credit counter against a ceiling.
///
/// [`CachePadded`] keeps the hot counter off neighbors' cache lines.
/// The `Backoff` in [`try_reserve`](Self::try_reserve) prevents a
/// contention storm under high thread counts (8+ concurrent reservers).
pub struct CreditGuard {
    /// Current bytes in flight. Written by reservers (CAS) and releasers
    /// (`fetch_sub`). `CachePadded` because multiple threads write it
    /// concurrently — without padding, the counter shares a cache line
    /// with `ceiling` (or adjacent struct fields in the consumer), causing
    /// false sharing on every `CAS/fetch_sub`.
    inflight: CachePadded<AtomicU64>,

    /// Maximum admissible bytes. Immutable after construction. Not padded
    /// because reads don't invalidate cache lines.
    ceiling: u64,
}

// CreditGuard is Send + Sync — all access is via atomics on a single counter.
// No unsafe impl needed: AtomicU64 is Send + Sync, CachePadded is Send + Sync.

// Explicit UnwindSafe impls. The auto-impl applies (CachePadded<AtomicU64>
// and u64 are both UnwindSafe), but explicit impls document the intent and
// prevent a future field from accidentally removing the trait.
impl UnwindSafe for CreditGuard {}
impl RefUnwindSafe for CreditGuard {}

impl CreditGuard {
    /// Create a credit guard with the given ceiling (maximum admissible bytes).
    ///
    /// # Panics
    ///
    /// Panics if `ceiling` is zero.
    pub fn new(ceiling: u64) -> Self {
        assert!(ceiling > 0, "credit ceiling must be non-zero");
        Self {
            inflight: CachePadded::new(AtomicU64::new(0)),
            ceiling,
        }
    }

    /// Try to reserve `n` bytes. Returns `true` if the reservation succeeded
    /// (inflight + n ≤ ceiling after the reservation).
    ///
    /// On failure (would exceed ceiling), returns `false` immediately — the
    /// caller must shed the item (IPC: `BULK_NACK(MemoryPressure)`).
    ///
    /// On CAS contention (another thread modified `inflight` between our
    /// load and CAS), retries with exponential backoff via
    /// [`Backoff::spin()`](crossbeam_utils::Backoff::spin). The backoff
    /// executes PAUSE instructions (1, 2, 4, 8, 16, 32, 64 — caps at 64
    /// per call after step 6) to reduce cache-line contention. This is the
    /// crossbeam `ArrayQueue::push`/`pop` pattern.
    ///
    /// `checked_add` prevents u64 overflow on adversarial `n`.
    pub fn try_reserve(&self, n: u64) -> bool {
        let backoff = Backoff::new();
        // Initial load: Relaxed is sufficient. The CAS failure path returns
        // the fresh actual value regardless of ordering. Acquire here would
        // add an unnecessary LDAR barrier on ARM64. This matches crossbeam's
        // ArrayQueue::push (Relaxed initial tail load) and ArrayQueue::pop
        // (Relaxed initial head load).
        let mut cur = self.inflight.load(Ordering::Relaxed);
        loop {
            // checked_add: an adversarial n that wraps u64 returns false.
            match cur.checked_add(n) {
                Some(sum) if sum <= self.ceiling => {}
                _ => return false,
            }
            match self.inflight.compare_exchange_weak(
                cur,
                cur + n,
                // Success: AcqRel. The Acquire half ensures the caller sees
                // all writes that happened before previous Release stores
                // (i.e., previous release() calls). The Release half ensures
                // the caller's subsequent use of the admitted item is visible
                // to the next release()'s reader. AcqRel (not SeqCst) because
                // CreditGuard has a single counter — no cross-variable
                // coordination requiring total order.
                Ordering::AcqRel,
                // Failure: Relaxed. The CAS returns the fresh actual value
                // in `actual` regardless of the failure ordering. Acquire
                // here would add a pointless LDAR on ARM64. Matches
                // ArrayQueue::push failure ordering (Relaxed) and
                // ArrayQueue::pop failure ordering (Relaxed).
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => {
                    cur = actual;
                    backoff.spin();
                }
            }
        }
    }

    /// Release `n` bytes. Called when the admitted item is delivered or
    /// dropped.
    ///
    /// `Release` ordering so the freed credit is visible to the next
    /// reserver's `Acquire` load (the Acquire half of the successful CAS).
    /// `AcqRel` is not needed because `release` does not need to observe
    /// the counter's previous value — it only needs to make the decrement
    /// visible.
    ///
    /// # Panics (debug builds only)
    ///
    /// Debug-asserts that `n <= inflight`. In release builds, an underflow
    /// wraps silently (the counter becomes `u64::MAX - delta`), permanently
    /// corrupting the guard. Callers must ensure `release(n)` is called
    /// exactly once per successful `try_reserve(n)` with the same `n`.
    #[inline]
    pub fn release(&self, n: u64) {
        debug_assert!(
            n <= self.inflight.load(Ordering::Relaxed),
            "CreditGuard::release({n}) exceeds inflight {} — \
             caller released more than it reserved",
            self.inflight.load(Ordering::Relaxed),
        );
        self.inflight.fetch_sub(n, Ordering::Release);
    }

    /// Current bytes in flight. `Relaxed` — this is an observability gauge,
    /// not a synchronization point. The value may be stale.
    #[inline]
    pub fn inflight(&self) -> u64 {
        self.inflight.load(Ordering::Relaxed)
    }

    /// The ceiling (maximum admissible bytes). Immutable after construction.
    #[inline]
    pub fn ceiling(&self) -> u64 {
        self.ceiling
    }

    /// Approximate headroom: `ceiling - inflight`. May be stale.
    #[inline]
    pub fn headroom(&self) -> u64 {
        self.ceiling.saturating_sub(self.inflight())
    }
}

impl fmt::Debug for CreditGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreditGuard")
            .field("inflight", &self.inflight())
            .field("ceiling", &self.ceiling)
            .field("headroom", &self.headroom())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn new_starts_empty() {
        let guard = CreditGuard::new(100);
        assert_eq!(guard.inflight(), 0);
        assert_eq!(guard.ceiling(), 100);
        assert_eq!(guard.headroom(), 100);
    }

    #[test]
    #[should_panic(expected = "non-zero")]
    fn new_rejects_zero_ceiling() {
        let _guard = CreditGuard::new(0);
    }

    #[test]
    fn reserve_and_release() {
        let guard = CreditGuard::new(100);
        assert!(guard.try_reserve(30));
        assert_eq!(guard.inflight(), 30);
        assert_eq!(guard.headroom(), 70);

        assert!(guard.try_reserve(70));
        assert_eq!(guard.inflight(), 100);
        assert_eq!(guard.headroom(), 0);

        // At ceiling — next reserve fails.
        assert!(!guard.try_reserve(1));

        guard.release(50);
        assert_eq!(guard.inflight(), 50);
        assert!(guard.try_reserve(50));
    }

    #[test]
    fn overflow_protection() {
        let guard = CreditGuard::new(100);
        // An adversarial n that would wrap u64 is rejected.
        assert!(!guard.try_reserve(u64::MAX));
        assert_eq!(guard.inflight(), 0);

        // Even after a reserve, a wrapping n is rejected.
        assert!(guard.try_reserve(50));
        assert!(!guard.try_reserve(u64::MAX));
        assert_eq!(guard.inflight(), 50);
    }

    #[test]
    fn exact_ceiling() {
        let guard = CreditGuard::new(10);
        assert!(guard.try_reserve(10));
        assert!(!guard.try_reserve(1));
        guard.release(1);
        assert!(guard.try_reserve(1));
    }

    #[test]
    fn concurrent_reserve_exactly_ceiling() {
        use std::sync::Arc;
        use std::thread;

        let guard = Arc::new(CreditGuard::new(10));
        let mut handles = Vec::new();

        for _ in 0..20 {
            let g = Arc::clone(&guard);
            handles.push(thread::spawn(move || g.try_reserve(1)));
        }

        let successes: usize = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|&ok| ok)
            .count();

        // The CAS loop guarantees no lost updates on a single counter.
        // With ceiling=10 and 20 threads each reserving 1, exactly 10
        // must succeed — no more (ceiling violation), no fewer (lost CAS).
        assert_eq!(
            successes, 10,
            "expected exactly 10 successful reserves with ceiling 10, got {successes}"
        );
        assert_eq!(
            guard.inflight(),
            10,
            "inflight must equal ceiling after exactly ceiling reservations"
        );
    }

    #[test]
    fn release_after_concurrent_reserve() {
        use std::sync::Arc;
        use std::thread;

        let guard = Arc::new(CreditGuard::new(5));

        // Reserve all 5.
        for _ in 0..5 {
            assert!(guard.try_reserve(1));
        }
        assert!(!guard.try_reserve(1));

        // Release from another thread, then verify headroom.
        let g = Arc::clone(&guard);
        thread::spawn(move || g.release(3))
            .join()
            .unwrap();

        assert_eq!(guard.inflight(), 2);
        assert_eq!(guard.headroom(), 3);
        assert!(guard.try_reserve(3));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "exceeds inflight")]
    fn release_underflow_panics_in_debug() {
        let guard = CreditGuard::new(100);
        guard.try_reserve(10);
        // Release more than reserved — debug_assert fires.
        guard.release(20);
    }

    #[test]
    fn single_byte_reserve_release_cycle() {
        let guard = CreditGuard::new(1);
        for _ in 0..1000 {
            assert!(guard.try_reserve(1));
            assert!(!guard.try_reserve(1));
            guard.release(1);
        }
        assert_eq!(guard.inflight(), 0);
    }

    #[test]
    fn debug_format() {
        let guard = CreditGuard::new(256);
        guard.try_reserve(100);
        let debug = format!("{guard:?}");
        assert!(debug.contains("CreditGuard"));
        assert!(debug.contains("inflight: 100"));
        assert!(debug.contains("ceiling: 256"));
        assert!(debug.contains("headroom: 156"));
    }
}
