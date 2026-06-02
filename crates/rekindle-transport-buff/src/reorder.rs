//! [`ReorderRing`] — fixed-capacity, sequence-keyed reorder buffer.
//!
//! Many producers fill slots out of order by sequence number. One consumer
//! drains the contiguous prefix in order. Allocates backing store once at
//! construction; reuses slots. Zero per-item allocation in the steady state.
//!
//! This is the direct replacement for `BTreeMap`-based reorder buffers
//! (O(log n) per insert, allocation per node) with an O(1) per insert,
//! zero-allocation ring. The fix for the ~50 MB throughput knee.
//!
//! # Architecture
//!
//! ```text
//!  window = 8 (mask = 7), next_deliver = 4
//!  ┌────┬────┬────┬────┬────┬────┬────┬────┐
//!  │ E  │ E  │ E  │ E  │ F4 │ F5 │ E  │ F7 │
//!  └────┴────┴────┴────┴────┴────┴────┴────┘
//!                        ▲ next_deliver
//!  drain: seq4 → deliver, seq5 → deliver, seq6 EMPTY → stop
//! ```
//!
//! # Memory orderings
//!
//! The protocol is derived from the crossbeam `ArrayQueue` stamp protocol
//! and the disruptor-rs `Cursor` pattern, adapted for the MP-fill / SC-drain
//! case where the producer's slot is determined by `seq & mask` (no CAS claim).
//!
//! - **Producer publish:** `with_mut` writes to seq and item, then
//!   `state.store(FILLED, Release)`. The Release store is the publish fence.
//! - **Consumer drain:** `state.load(Acquire)` pairs with the producer's
//!   Release. `with_mut` reads seq and item. Then `state.store(EMPTY, Release)`
//!   resets the slot for reuse — this Release pairs with the next producer's
//!   Acquire load of EMPTY.
//! - **`window_base`:** stored with `Release` *before* the `EMPTY` Release
//!   store on the slot state, so a producer that observes `EMPTY` via Acquire
//!   also sees the updated `window_base`. Loaded with `Acquire` by producers
//!   for the overflow check.
//!
//! Every ordering is loom-verified in `tests/loom_reorder.rs`.
//!
//! # Idempotency
//!
//! `publish` is **not** idempotent. Publishing the same `seq` twice returns
//! [`PublishError::Occupied`]. Duplicate detection (via `ContentHash` or
//! similar) is the consumer's responsibility — the ring does not deduplicate.
//! This is a deliberate design choice: each chunk in a bulk transfer has a
//! unique sequence number, and a duplicate is a protocol error that the
//! consumer must surface, not silently absorb.

use core::cell::Cell;
use core::fmt;
use core::mem::{self, MaybeUninit};
use core::panic::{RefUnwindSafe, UnwindSafe};
use core::ptr;

use crossbeam_utils::CachePadded;

use crate::loom_shim::cell::UnsafeCell;
use crate::loom_shim::sync::atomic::{AtomicUsize, Ordering};

// window_base is stored as AtomicUsize. On 32-bit targets, usize is 32 bits
// and the `new_deliver as usize` cast silently truncates after 2^32 deliveries.
// This crate targets 64-bit platforms (Linux 6.12+ on x86-64/aarch64).
// A 32-bit build is a configuration error caught here at compile time.
#[cfg(not(target_pointer_width = "64"))]
compile_error!(
    "rekindle-transport-buff requires a 64-bit target. \
     window_base uses AtomicUsize which truncates u64 on 32-bit platforms."
);

/// Slot state: empty, available for a producer to write.
const EMPTY: usize = 0;
/// Slot state: filled by a producer, available for the consumer to read.
const FILLED: usize = 1;

// ---------------------------------------------------------------------------
// Slot<T>
// ---------------------------------------------------------------------------

/// One slot in the ring. Each slot's `state` is on its own cache line via
/// [`CachePadded`] because multiple producers write adjacent slots
/// simultaneously — without padding, their stores ping-pong the same cache
/// line (the false-sharing cost that flattens scaling).
///
/// `seq` and `item` share a cache line with each other — that is correct:
/// only one producer writes them (the one whose sequence maps to this slot),
/// protected by the state gate.
struct Slot<T> {
    /// Atomic state: [`EMPTY`] or [`FILLED`].
    ///
    /// `CachePadded` to eliminate false sharing under concurrent multi-producer
    /// publish. Without padding, two producers writing `slots[0].state` and
    /// `slots[1].state` ping-pong the same 64/128-byte cache line — the USL β
    /// coherency cost that flattens scaling.
    ///
    /// Note: crossbeam's `ArrayQueue` does NOT pad per-slot stamps because its
    /// CAS serializes access to each slot. The `ReorderRing` has no CAS — the
    /// producer knows its slot from `seq & mask` — so adjacent slots can be
    /// written concurrently. Padding is a performance optimization (not a
    /// correctness requirement): the ring is correct without it, but scaling
    /// degrades under concurrent multi-producer load without it.
    state: CachePadded<AtomicUsize>,

    /// The sequence number this slot currently holds. Written by the producer
    /// under the `state == EMPTY` gate; read by the consumer under the
    /// `state == FILLED` gate. Not atomic — protected by the state protocol.
    seq: UnsafeCell<u64>,

    /// The payload. Written by the producer, read (moved out) by the consumer.
    /// Same protection as `seq`.
    item: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Slot<T> {
    fn new() -> Self {
        Self {
            state: CachePadded::new(AtomicUsize::new(EMPTY)),
            seq: UnsafeCell::new(0),
            item: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

// ---------------------------------------------------------------------------
// PublishError
// ---------------------------------------------------------------------------

/// Error returned by [`ReorderRing::publish`].
pub enum PublishError<T> {
    /// The sequence is beyond the current window (`seq >= window_base + window`)
    /// or below the current base (`seq < window_base`).
    /// The consumer has not advanced far enough (or has already passed this seq).
    /// The item is returned so the caller can retry or propagate backpressure.
    ///
    /// The ring does **not** interpret this — the consumer decides whether
    /// overflow is backpressure, a flood, or an attack.
    Overflow {
        /// The item that could not be published.
        item: T,
        /// The current window upper bound (exclusive): `window_base + window`.
        high: u64,
    },

    /// The slot is already filled (state == FILLED). Either:
    /// - A duplicate sequence (the caller's dedup should have caught this), or
    /// - A window-discipline violation (the slot holds a different sequence
    ///   from a previous cycle that was never drained).
    ///
    /// The item is returned.
    Occupied {
        /// The item that could not be published.
        item: T,
    },
}

impl<T> fmt::Debug for PublishError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow { high, .. } => {
                f.debug_struct("Overflow").field("high", high).finish()
            }
            Self::Occupied { .. } => f.debug_struct("Occupied").finish(),
        }
    }
}

impl<T> PublishError<T> {
    /// Extract the item that could not be published.
    pub fn into_inner(self) -> T {
        match self {
            Self::Overflow { item, .. } | Self::Occupied { item } => item,
        }
    }
}

// ---------------------------------------------------------------------------
// ReorderRing<T>
// ---------------------------------------------------------------------------

/// A fixed-capacity, sequence-keyed reorder buffer.
///
/// Many producers fill slots out of order via [`publish`](Self::publish).
/// One consumer drains the contiguous prefix in order via
/// [`drain_contiguous`](Self::drain_contiguous). The backing store is
/// allocated once; slots are reused. Zero per-item allocation in the
/// steady state.
///
/// `T` may be `!Send` — the ring does not impose `Send` on `T`. The
/// `unsafe impl Send/Sync` bounds require `T: Send` explicitly.
///
/// # Panics
///
/// [`ReorderRing::new`] panics if `window` is zero or not a power of two.
pub struct ReorderRing<T> {
    /// The slot array. Length = `window`, a power of two.
    slots: Box<[Slot<T>]>,

    /// `window - 1`. Used to mask sequence numbers to slot indices.
    mask: u64,

    /// The next sequence the consumer expects to deliver. Single-writer
    /// (consumer only). Not atomic — cross-thread visibility rides the
    /// per-slot Release/Acquire, not this cursor. See the module doc for
    /// the ordering derivation.
    ///
    /// If observability from another thread is needed, read `window_base`
    /// (which mirrors this value with Release after each drain step).
    next_deliver: Cell<u64>,

    /// Count of items currently stored in FILLED state. Single-writer
    /// (incremented by producers under the state gate, decremented by
    /// the consumer in drain). Used for O(1) `has_buffered()` check
    /// (the mac80211 `stored_mpdu_num` pattern).
    ///
    /// This is an `AtomicUsize` because producers and the consumer
    /// both modify it. Producers increment with `Relaxed` (the state
    /// gate on the slot provides the actual synchronization). The
    /// consumer decrements with `Relaxed` and reads with `Relaxed`
    /// (observability, not synchronization).
    stored_count: AtomicUsize,

    /// A mirror of `next_deliver` that producers read (with `Acquire`) for
    /// the overflow check. Updated by the consumer with `Release` *before*
    /// the slot's `state.store(EMPTY, Release)`, so that a producer
    /// observing `EMPTY` via Acquire is guaranteed to also see the updated
    /// `window_base`.
    ///
    /// Stored *before* the EMPTY Release store: the Release on EMPTY is a
    /// one-way barrier that prevents earlier stores from being reordered
    /// after it, so `window_base` (stored before) is in the happens-before
    /// set visible to any thread that Acquire-loads EMPTY.
    window_base: CachePadded<AtomicUsize>,
}

// SAFETY: The ring is safe to share across threads when T: Send.
//
// Sync: `publish` takes `&self` and is safe for concurrent callers because
// each producer writes a distinct slot (determined by `seq & mask`) and the
// per-slot `state` atomic gates exclusive access. `drain_contiguous` takes
// `&self` and is safe because it is only called by one consumer (the
// single-writer contract on `next_deliver`). This is a usage contract, not
// enforced by the type system — the consumer must ensure single-writer.
//
// Send: the ring can be moved to another thread. The `Cell<u64>` is !Sync
// but that is fine — we implement Sync manually with the safety argument above.
unsafe impl<T: Send> Send for ReorderRing<T> {}
// SAFETY: see above.
unsafe impl<T: Send> Sync for ReorderRing<T> {}

impl<T> UnwindSafe for ReorderRing<T> {}
impl<T> RefUnwindSafe for ReorderRing<T> {}

impl<T> ReorderRing<T> {
    /// Create a new ring with the given window size, starting at seq 0.
    ///
    /// # Panics
    ///
    /// Panics if `window` is zero or not a power of two.
    pub fn new(window: usize) -> Self {
        Self::new_with_base(window, 0)
    }

    /// Create a new ring starting at an arbitrary base sequence.
    ///
    /// The first `publish` must use `seq >= start_seq`. The first
    /// `drain_contiguous` delivers from `start_seq`. This is the
    /// resume-after-reconnect constructor: after a `BULK_RESUME`
    /// at `last_acked_chunk_index`, the receiver creates the ring
    /// with `start_seq = last_acked_chunk_index + 1`.
    ///
    /// Also used to test near-`u64::MAX` sequence numbers without
    /// draining billions of slots.
    ///
    /// # Panics
    ///
    /// Panics if `window` is zero or not a power of two.
    pub fn new_with_base(window: usize, start_seq: u64) -> Self {
        assert!(window > 0, "window must be non-zero");
        assert!(
            window.is_power_of_two(),
            "window must be a power of two, got {window}"
        );

        let slots: Box<[Slot<T>]> = (0..window).map(|_| Slot::new()).collect();
        let mask = (window as u64) - 1;

        Self {
            slots,
            mask,
            next_deliver: Cell::new(start_seq),
            stored_count: AtomicUsize::new(0),
            window_base: CachePadded::new(AtomicUsize::new(start_seq as usize)),
        }
    }

    /// Publish `item` at sequence number `seq`.
    ///
    /// Returns `Ok(())` on success. Returns `Err` with the item if:
    /// - `seq` is beyond the window (`Overflow`)
    /// - the target slot is already filled (`Occupied`)
    ///
    /// # Ordering
    ///
    /// 1. Load `window_base` with `Acquire` — pairs with consumer's
    ///    `Release` store of `window_base` in `drain_contiguous`.
    /// 2. Load `slot.state` with `Acquire` — pairs with consumer's
    ///    `Release` store of `EMPTY` in `drain_contiguous`.
    /// 3. Write `seq` and `item` via `UnsafeCell` (protected by state gate).
    /// 4. Store `slot.state = FILLED` with `Release` — the publish fence.
    ///    Everything written before (seq, item) is visible to any thread
    ///    that observes `FILLED` with `Acquire`.
    pub fn publish(&self, seq: u64, item: T) -> Result<(), PublishError<T>> {
        // --- Overflow check ---
        // Load window_base with Acquire. The consumer stores it with Release
        // before storing EMPTY with Release, so if we later see EMPTY on the
        // slot via Acquire, we are guaranteed to see the window_base that was
        // current at that drain step.
        let base = self.window_base.load(Ordering::Acquire) as u64;
        let window = self.window() as u64;
        // Compute the distance from base without overflow. If seq < base,
        // checked_sub returns None → below-window. If seq - base >= window,
        // it's above-window. This avoids computing base + window which
        // overflows near u64::MAX.
        let Some(distance) = seq.checked_sub(base) else {
            // seq < base: the consumer already delivered this seq.
            let high = base.saturating_add(window);
            return Err(PublishError::Overflow { item, high });
        };
        if distance >= window {
            let high = base.saturating_add(window);
            return Err(PublishError::Overflow { item, high });
        }

        let idx = (seq & self.mask) as usize;
        // SAFETY: idx is within bounds — guaranteed by the mask
        // (`idx < window` because `mask = window - 1` and window is a
        // power of two).
        debug_assert!(idx < self.slots.len());
        // SAFETY: idx < slots.len() guaranteed by mask (idx = seq & (window-1), window = slots.len()).
        let slot = unsafe { self.slots.get_unchecked(idx) };

        // --- State check ---
        // Acquire pairs with the consumer's Release store of EMPTY.
        let state = slot.state.load(Ordering::Acquire);

        if state != EMPTY {
            return Err(PublishError::Occupied { item });
        }

        // --- Write data ---
        // state == EMPTY guarantees no concurrent access to this slot's data.
        // The producer has exclusive write access until it stores FILLED.
        slot.seq
            // SAFETY: exclusive access via state gate; pointer does not escape closure.
            .with_mut(|ptr| unsafe { ptr::write(ptr, seq) });
        slot.item
            // SAFETY: exclusive access via state gate; pointer does not escape closure.
            .with_mut(|ptr| unsafe { ptr::write(ptr, MaybeUninit::new(item)) });

        // --- Publish ---
        // Release store. Everything written before (seq, item) is visible to
        // any thread that observes FILLED with Acquire.
        slot.state.store(FILLED, Ordering::Release);

        // Increment stored count. Relaxed: the actual synchronization is
        // carried by the slot state Release/Acquire, not this counter.
        // This counter is for O(1) has_buffered() checks.
        self.stored_count.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Drain and deliver every contiguous filled slot starting at
    /// `next_deliver`, calling `f` for each in sequence order, until a gap.
    ///
    /// Returns the number of items delivered. This is the **batched drain** —
    /// the consumer calls it once per wake and it delivers the whole contiguous
    /// prefix, which is the batching that eliminates per-item serialization.
    ///
    /// # Safety contract (usage, not `unsafe`)
    ///
    /// This method must be called by exactly one thread (the single-writer
    /// consumer). Concurrent calls from multiple threads are a data race on
    /// `next_deliver`. This is a usage contract — the type system does not
    /// enforce it (the ring is `Sync` for producer access). The consumer
    /// must hold this invariant.
    ///
    /// # Ordering
    ///
    /// 1. Load `slot.state` with `Acquire` — pairs with producer's `Release`
    ///    store of `FILLED`.
    /// 2. Read `seq` and `item` via `UnsafeCell` — safe because `FILLED`
    ///    guarantees the producer has finished writing.
    /// 3. Move the item out via `ptr::read` — ownership transfers to `f`.
    /// 4. Advance `next_deliver` and `window_base`.
    /// 5. Store `window_base` with `Release` — must happen *before* the
    ///    `EMPTY` Release store so that a producer observing `EMPTY` via
    ///    Acquire is guaranteed to see the updated `window_base`.
    /// 6. Store `slot.state = EMPTY` with `Release` — pairs with the next
    ///    producer's `Acquire` load. This Release is a one-way barrier:
    ///    the `window_base` Release store (step 5) cannot be reordered
    ///    after it, so both are in the happens-before set visible to the
    ///    producer's Acquire.
    /// 7. Call `f(seq, item)`. The slot is already reset — the ring is in a
    ///    consistent state during the callback.
    pub fn drain_contiguous(&self, mut f: impl FnMut(u64, T)) -> usize {
        let mut count: usize = 0;

        loop {
            let seq = self.next_deliver.get();
            let idx = (seq & self.mask) as usize;

            debug_assert!(idx < self.slots.len());
            // SAFETY: idx < slots.len() guaranteed by mask.
            let slot = unsafe { self.slots.get_unchecked(idx) };

            // Acquire pairs with producer's Release store of FILLED.
            if slot.state.load(Ordering::Acquire) != FILLED {
                break;
            }

            // State == FILLED guarantees the producer finished writing seq.
            let slot_seq = slot.seq
                // SAFETY: FILLED state + Acquire load establishes happens-before.
                .with(|ptr| unsafe { ptr::read(ptr) });

            if slot_seq != seq {
                break;
            }

            // Move item out. ptr::read transfers ownership; slot is logically
            // uninitialized after this.
            let item: T = slot.item
                // SAFETY: FILLED + seq match guarantees item is initialized.
                .with_mut(|ptr| unsafe { ptr::read(ptr).assume_init() });

            // --- Advance cursors ---
            // wrapping_add: at u64::MAX the consumer wraps to 0. This is
            // correct: the ring is indexed by seq & mask, so the slot index
            // wraps. The test ring_publish_at_max_u64_seq exercises this.
            let new_deliver = seq.wrapping_add(1);
            self.next_deliver.set(new_deliver);

            // --- Update window_base BEFORE EMPTY store ---
            // Release: ensures the updated window_base is in the happens-before
            // set visible to any producer that subsequently Acquire-loads the
            // slot's EMPTY state.
            self.window_base
                .store(new_deliver as usize, Ordering::Release);

            // --- Reset slot ---
            // Release: pairs with the next producer's Acquire load of EMPTY.
            slot.state.store(EMPTY, Ordering::Release);

            // Decrement stored count. Relaxed: observability counter.
            self.stored_count.fetch_sub(1, Ordering::Relaxed);

            // --- Deliver ---
            f(seq, item);

            count += 1;
        }

        count
    }

    /// Force-advance `next_deliver` to `exclusive_seq`, delivering filled
    /// slots and skipping gaps.
    ///
    /// For each sequence in `[next_deliver, exclusive_seq)`:
    /// - If the slot is FILLED with the correct seq: calls `f(seq, Some(item))`
    /// - If the slot is EMPTY or holds a wrong seq: calls `f(seq, None)`
    ///
    /// This is the `dispatch_pkt_until_start_win` equivalent from the
    /// mac80211 and mwifiex kernel drivers. Use it on session teardown
    /// or timeout when a sender has crashed mid-stream and the contiguous
    /// prefix will never complete naturally.
    ///
    /// # Safety contract (usage, not `unsafe`)
    ///
    /// Same single-writer contract as `drain_contiguous`.
    ///
    /// # Panics
    ///
    /// Panics if `exclusive_seq < next_deliver` (cannot rewind).
    pub fn drain_until(&self, exclusive_seq: u64, mut f: impl FnMut(u64, Option<T>)) -> usize {
        assert!(
            exclusive_seq >= self.next_deliver.get(),
            "drain_until: cannot rewind (exclusive_seq={exclusive_seq} < next_deliver={})",
            self.next_deliver.get()
        );

        let mut count: usize = 0;

        while self.next_deliver.get() < exclusive_seq {
            let seq = self.next_deliver.get();
            let idx = (seq & self.mask) as usize;

            debug_assert!(idx < self.slots.len());
            // SAFETY: idx < slots.len() guaranteed by mask.
            let slot = unsafe { self.slots.get_unchecked(idx) };

            let state = slot.state.load(Ordering::Acquire);

            let item = if state == FILLED {
                // SAFETY: FILLED + Acquire establishes happens-before for seq read.
                let slot_seq = slot.seq.with(|ptr| unsafe { ptr::read(ptr) });
                if slot_seq == seq {
                    let item: T = slot.item
                        // SAFETY: FILLED + seq match guarantees item is initialized.
                        .with_mut(|ptr| unsafe { ptr::read(ptr).assume_init() });
                    self.stored_count.fetch_sub(1, Ordering::Relaxed);
                    Some(item)
                } else {
                    if mem::needs_drop::<T>() {
                        slot.item.with_mut(|ptr|
                            // SAFETY: FILLED guarantees item is initialized; dropping stale entry.
                            unsafe { (*ptr).assume_init_drop() });
                        self.stored_count.fetch_sub(1, Ordering::Relaxed);
                    }
                    None
                }
            } else {
                // EMPTY slot — gap. Skip it.
                None
            };

            // Advance. wrapping_add: same reasoning as drain_contiguous.
            let new_deliver = seq.wrapping_add(1);
            self.next_deliver.set(new_deliver);
            self.window_base
                .store(new_deliver as usize, Ordering::Release);
            slot.state.store(EMPTY, Ordering::Release);

            f(seq, item);
            count += 1;
        }

        count
    }

    /// The next sequence the consumer expects to deliver.
    ///
    /// Only meaningful when called from the consumer thread.
    #[inline]
    pub fn next_deliver(&self) -> u64 {
        self.next_deliver.get()
    }

    /// The current window upper bound (exclusive): sequences
    /// `[next_deliver, window_high)` are admissible.
    #[inline]
    pub fn window_high(&self) -> u64 {
        self.next_deliver.get().saturating_add(self.window() as u64)
    }

    /// Whether the next delivery slot is ready (FILLED with the correct seq).
    ///
    /// Only meaningful when called from the consumer thread. This checks
    /// only the slot at `next_deliver` — it does NOT scan the ring. Use
    /// [`has_buffered`](Self::has_buffered) to check if any items are
    /// stored anywhere in the ring.
    #[inline]
    pub fn next_is_ready(&self) -> bool {
        let idx = (self.next_deliver.get() & self.mask) as usize;
        debug_assert!(idx < self.slots.len());
        // SAFETY: idx < slots.len() guaranteed by mask.
        let slot = unsafe { self.slots.get_unchecked(idx) };
        slot.state.load(Ordering::Acquire) == FILLED
    }

    /// Whether any items are currently stored in the ring (FILLED state).
    ///
    /// O(1) via the `stored_count` counter (the mac80211 `stored_mpdu_num`
    /// pattern). Relaxed load — this is an approximation that may be
    /// briefly stale but converges.
    #[inline]
    pub fn has_buffered(&self) -> bool {
        self.stored_count.load(Ordering::Relaxed) > 0
    }

    /// The number of items currently stored in the ring (FILLED state).
    ///
    /// Relaxed load — observability approximation. May be briefly stale.
    #[inline]
    pub fn stored_count(&self) -> usize {
        self.stored_count.load(Ordering::Relaxed)
    }

    /// Whether no items are stored and the next delivery slot is empty.
    ///
    /// Equivalent to `!has_buffered()`. Note: this can return `true` even
    /// when `next_is_ready()` returns `true` if the `stored_count` Relaxed
    /// load is stale. For the consumer thread, prefer `next_is_ready()`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        !self.has_buffered()
    }

    /// In-order fast path: if `seq` equals `next_deliver` and the ring
    /// is empty (`stored_count == 0`), the item can be delivered directly
    /// to the consumer without touching the slot array.
    ///
    /// Returns `Ok(item)` if the fast path applies (the item is consumed
    /// and `next_deliver` is advanced). Returns `Err(item)` if the fast
    /// path does not apply — the caller must fall back to
    /// `publish` + `drain_contiguous`.
    ///
    /// The fast path bypasses the slot state machine entirely for in-order
    /// delivery when nothing is buffered. The `stored_count` check uses
    /// `Acquire` ordering. Producers increment `stored_count` with
    /// `Relaxed` inside `publish()`, so the Acquire/Relaxed pair is
    /// advisory — a stale 0 is possible on weakly-ordered hardware.
    ///
    /// A stale 0 is **safe**: if the consumer sees 0 when a producer has
    /// actually published seq M (where M > `next_deliver`, since it's
    /// out-of-order), the fast path delivers seq N (== `next_deliver`)
    /// directly. Seq M is still in the ring. The caller's subsequent
    /// `drain_contiguous` will deliver seq M when the contiguous prefix
    /// reaches it. Delivery order is preserved because the buffered item
    /// always has a higher seq than the fast-path-delivered item.
    ///
    /// A false negative (`stored_count` > 0 but all items are for seqs
    /// the consumer hasn't reached yet) causes the fast path to fall
    /// back to publish+drain unnecessarily — a performance miss, not
    /// a correctness bug.
    ///
    /// # Precondition
    ///
    /// `seq` must not already be published to the ring. If `seq` is
    /// already in a slot (published by a producer independently), the
    /// fast path delivers the new item and the slot item is orphaned.
    /// In the IPC use case this cannot happen because seq is assigned
    /// by the consumer before dispatch — no producer publishes a seq
    /// the consumer has not assigned.
    ///
    /// # Safety contract (usage, not `unsafe`)
    ///
    /// Same single-writer contract as `drain_contiguous` — only the consumer
    /// thread may call this, because it reads and writes `next_deliver`.
    #[inline]
    pub fn try_deliver_direct(&self, seq: u64, item: T) -> Result<T, T> {
        if seq == self.next_deliver.get()
            && self.stored_count.load(Ordering::Acquire) == 0
        {
            let new_deliver = seq.wrapping_add(1);
            self.next_deliver.set(new_deliver);
            self.window_base
                .store(new_deliver as usize, Ordering::Release);
            Ok(item)
        } else {
            Err(item)
        }
    }

    /// The window size (number of slots).
    #[inline]
    pub fn window(&self) -> usize {
        self.slots.len()
    }

    /// The current delivery base as seen by producers. This is an
    /// approximation — it may lag behind `next_deliver` by one drain step.
    /// Useful for observability from non-consumer threads.
    #[inline]
    pub fn delivery_base(&self) -> u64 {
        self.window_base.load(Ordering::Relaxed) as u64
    }
}

impl<T> Drop for ReorderRing<T> {
    fn drop(&mut self) {
        // Optimization: if T has no destructor, skip the entire loop.
        // This is a compile-time check — for T = u64, LinkInput, etc.,
        // the loop is elided entirely. Pattern from crossbeam ArrayQueue.
        if !mem::needs_drop::<T>() {
            return;
        }

        // We have &mut self — exclusive access. No concurrent producers.
        // Relaxed is correct: no cross-thread synchronization needed.
        for slot in &self.slots {
            if slot.state.load(Ordering::Relaxed) == FILLED {
                // SAFETY: state == FILLED means the item is initialized.
                // We have exclusive access (&mut self), so no concurrent
                // access is possible.
                slot.item.with_mut(|ptr| unsafe {
                    (*ptr).assume_init_drop();
                });
            }
        }
    }
}

impl<T> fmt::Debug for ReorderRing<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReorderRing")
            .field("window", &self.window())
            .field("next_deliver", &self.next_deliver.get())
            .field("stored_count", &self.stored_count())
            .field("delivery_base", &self.delivery_base())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Tests (non-loom, basic correctness)
// ---------------------------------------------------------------------------

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn new_power_of_two() {
        let ring = ReorderRing::<u64>::new(8);
        assert_eq!(ring.window(), 8);
        assert_eq!(ring.next_deliver(), 0);
        assert_eq!(ring.window_high(), 8);
        assert!(!ring.next_is_ready());
        assert!(!ring.has_buffered());
        assert!(ring.is_empty());
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn new_rejects_non_power_of_two() {
        let _ring = ReorderRing::<u64>::new(7);
    }

    #[test]
    #[should_panic(expected = "non-zero")]
    fn new_rejects_zero() {
        let _ring = ReorderRing::<u64>::new(0);
    }

    #[test]
    fn publish_and_drain_in_order() {
        let ring = ReorderRing::new(4);
        ring.publish(0, 100u64).unwrap();
        ring.publish(1, 200u64).unwrap();
        ring.publish(2, 300u64).unwrap();

        assert_eq!(ring.stored_count(), 3);

        let mut delivered = Vec::new();
        let count = ring.drain_contiguous(|seq, item| delivered.push((seq, item)));

        assert_eq!(count, 3);
        assert_eq!(delivered, vec![(0, 100), (1, 200), (2, 300)]);
        assert_eq!(ring.next_deliver(), 3);
        assert_eq!(ring.stored_count(), 0);
    }

    #[test]
    fn publish_out_of_order_delivers_in_order() {
        let ring = ReorderRing::new(8);
        ring.publish(2, 'c').unwrap();
        ring.publish(0, 'a').unwrap();
        ring.publish(1, 'b').unwrap();

        let mut delivered = Vec::new();
        ring.drain_contiguous(|seq, item| delivered.push((seq, item)));

        assert_eq!(delivered, vec![(0, 'a'), (1, 'b'), (2, 'c')]);
    }

    #[test]
    fn gap_holds_later_items() {
        let ring = ReorderRing::new(8);
        ring.publish(1, 10u64).unwrap();
        ring.publish(3, 30u64).unwrap();

        // Nothing contiguous from 0
        let count = ring.drain_contiguous(|_, _| {});
        assert_eq!(count, 0);
        assert!(!ring.next_is_ready()); // slot 0 is EMPTY
        assert!(ring.has_buffered());   // but items exist elsewhere

        // Fill the gap at 0
        ring.publish(0, 0u64).unwrap();
        let mut delivered = Vec::new();
        ring.drain_contiguous(|seq, item| delivered.push((seq, item)));
        assert_eq!(delivered, vec![(0, 0), (1, 10)]);
        assert_eq!(ring.next_deliver(), 2);
    }

    #[test]
    fn overflow_returns_item() {
        let ring = ReorderRing::<String>::new(4);
        let result = ring.publish(4, String::from("too far"));
        match result {
            Err(PublishError::Overflow { item, high }) => {
                assert_eq!(item, "too far");
                assert_eq!(high, 4);
            }
            other => panic!("expected Overflow, got {other:?}"),
        }
    }

    #[test]
    fn below_base_returns_overflow() {
        let ring = ReorderRing::new(4);
        ring.publish(0, 10u64).unwrap();
        ring.drain_contiguous(|_, _| {});
        let result = ring.publish(0, 20u64);
        assert!(matches!(result, Err(PublishError::Overflow { .. })));
    }

    #[test]
    fn occupied_returns_item_and_original_survives() {
        let ring = ReorderRing::new(4);
        ring.publish(0, 10u64).unwrap();
        let result = ring.publish(0, 20u64);
        match result {
            Err(PublishError::Occupied { item }) => {
                assert_eq!(item, 20);
            }
            other => panic!("expected Occupied, got {other:?}"),
        }
        // Verify the original item at seq=0 is still delivered correctly.
        let mut delivered = Vec::new();
        ring.drain_contiguous(|seq, item| delivered.push((seq, item)));
        assert_eq!(delivered, vec![(0, 10)]);
    }

    #[test]
    fn drain_stops_at_gap() {
        let ring = ReorderRing::new(8);
        ring.publish(0, 'a').unwrap();
        ring.publish(1, 'b').unwrap();
        ring.publish(3, 'd').unwrap();

        let count = ring.drain_contiguous(|_, _| {});
        assert_eq!(count, 2);
        assert_eq!(ring.next_deliver(), 2);
    }

    #[test]
    fn slot_reuse_after_drain() {
        let ring = ReorderRing::new(4);

        ring.publish(0, 100u64).unwrap();
        ring.drain_contiguous(|_, _| {});

        ring.publish(1, 101u64).unwrap();
        ring.publish(2, 102u64).unwrap();
        ring.publish(3, 103u64).unwrap();
        ring.drain_contiguous(|_, _| {});
        // next_deliver = 4, slot 0 is free again
        ring.publish(4, 200u64).unwrap();

        let mut delivered = Vec::new();
        ring.drain_contiguous(|seq, item| delivered.push((seq, item)));
        assert_eq!(delivered, vec![(4, 200)]);
    }

    #[test]
    fn overflow_then_drain_then_retry() {
        // Issue #3: verify that after overflow, draining, and retrying works.
        let ring = ReorderRing::<u64>::new(4);
        ring.publish(0, 0).unwrap();

        // Overflow: seq 4 is beyond window [0, 4)
        let err = ring.publish(4, 4);
        assert!(matches!(err, Err(PublishError::Overflow { .. })));

        // Drain seq 0 → next_deliver=1, window=[1, 5)
        ring.drain_contiguous(|_, _| {});

        // Now seq 4 is within window [1, 5)
        ring.publish(4, 4).unwrap();
        ring.publish(1, 1).unwrap();
        ring.publish(2, 2).unwrap();
        ring.publish(3, 3).unwrap();

        let mut delivered = Vec::new();
        ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
        assert_eq!(delivered, vec![(1, 1), (2, 2), (3, 3), (4, 4)]);
    }

    #[test]
    fn drop_cleans_filled_slots() {
        use std::sync::atomic::AtomicUsize as StdAtomicUsize;
        use std::sync::atomic::Ordering::Relaxed;

        static DROP_COUNT: StdAtomicUsize = StdAtomicUsize::new(0);

        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Relaxed);
            }
        }

        DROP_COUNT.store(0, Relaxed);
        {
            let ring = ReorderRing::new(4);
            ring.publish(0, Counted).unwrap();
            ring.publish(1, Counted).unwrap();
            ring.publish(2, Counted).unwrap();
        }
        assert_eq!(DROP_COUNT.load(Relaxed), 3);
    }

    #[test]
    fn needs_drop_optimization() {
        let ring = ReorderRing::new(4);
        ring.publish(0, 42u64).unwrap();
        ring.publish(1, 43u64).unwrap();
        drop(ring);
    }

    #[test]
    fn drain_returns_zero_on_empty() {
        let ring = ReorderRing::<u64>::new(4);
        assert_eq!(ring.drain_contiguous(|_, _| {}), 0);
    }

    #[test]
    fn publish_error_into_inner() {
        let ring = ReorderRing::new(4);
        ring.publish(0, 10u64).unwrap();
        let err = ring.publish(0, 20u64).unwrap_err();
        assert_eq!(err.into_inner(), 20);
    }

    #[test]
    fn debug_impl() {
        let ring = ReorderRing::<u64>::new(8);
        let debug = format!("{ring:?}");
        assert!(debug.contains("ReorderRing"));
        assert!(debug.contains("window: 8"));
        assert!(debug.contains("stored_count: 0"));
    }

    #[test]
    fn every_permutation_window_4() {
        let perms: Vec<[u64; 4]> = {
            let mut v = Vec::new();
            let items = [0u64, 1, 2, 3];
            let mut a = items;
            let n = a.len();
            let mut c = [0usize; 4];
            v.push(a);
            let mut i = 0;
            while i < n {
                if c[i] < i {
                    if i % 2 == 0 {
                        a.swap(0, i);
                    } else {
                        a.swap(c[i], i);
                    }
                    v.push(a);
                    c[i] += 1;
                    i = 0;
                } else {
                    c[i] = 0;
                    i += 1;
                }
            }
            v
        };

        for perm in &perms {
            let ring = ReorderRing::new(4);
            for &seq in perm {
                ring.publish(seq, seq * 10).unwrap();
            }
            let mut delivered = Vec::new();
            ring.drain_contiguous(|seq, item| delivered.push((seq, item)));
            assert_eq!(
                delivered,
                vec![(0, 0), (1, 10), (2, 20), (3, 30)],
                "failed for permutation {perm:?}"
            );
        }
    }

    #[test]
    fn has_buffered_with_gaps() {
        // Issue #5: is_empty/has_buffered must reflect items behind a gap.
        let ring = ReorderRing::new(8);
        assert!(!ring.has_buffered());
        assert!(ring.is_empty());

        // Publish seq 1 (gap at 0) — items ARE buffered, but next is not ready.
        ring.publish(1, 10u64).unwrap();
        assert!(ring.has_buffered());
        assert!(!ring.is_empty());
        assert!(!ring.next_is_ready());

        // Fill the gap.
        ring.publish(0, 0u64).unwrap();
        assert!(ring.has_buffered());
        assert!(ring.next_is_ready());

        // Drain all.
        ring.drain_contiguous(|_, _| {});
        assert!(!ring.has_buffered());
        assert!(ring.is_empty());
    }

    #[test]
    fn stored_count_tracks_correctly() {
        let ring = ReorderRing::new(8);
        assert_eq!(ring.stored_count(), 0);

        ring.publish(0, 'a').unwrap();
        assert_eq!(ring.stored_count(), 1);

        ring.publish(2, 'c').unwrap();
        assert_eq!(ring.stored_count(), 2);

        ring.publish(1, 'b').unwrap();
        assert_eq!(ring.stored_count(), 3);

        ring.drain_contiguous(|_, _| {});
        assert_eq!(ring.stored_count(), 0);
    }

    #[test]
    fn drain_until_with_gaps() {
        // Issue #2: force-advance past gaps on sender crash.
        let ring = ReorderRing::new(8);
        ring.publish(0, 'a').unwrap();
        // gap at 1
        ring.publish(2, 'c').unwrap();
        ring.publish(3, 'd').unwrap();
        // gap at 4

        let mut results = Vec::new();
        let count = ring.drain_until(5, |seq, item| results.push((seq, item)));
        assert_eq!(count, 5);
        assert_eq!(results[0], (0, Some('a')));
        assert_eq!(results[1], (1, None));       // gap
        assert_eq!(results[2], (2, Some('c')));
        assert_eq!(results[3], (3, Some('d')));
        assert_eq!(results[4], (4, None));       // gap
        assert_eq!(ring.next_deliver(), 5);
        assert_eq!(ring.stored_count(), 0);
    }

    #[test]
    fn drain_until_no_gaps() {
        let ring = ReorderRing::new(8);
        ring.publish(0, 10u64).unwrap();
        ring.publish(1, 20u64).unwrap();
        ring.publish(2, 30u64).unwrap();

        let mut delivered = Vec::new();
        ring.drain_until(3, |seq, item| {
            delivered.push((seq, item.unwrap()));
        });
        assert_eq!(delivered, vec![(0, 10), (1, 20), (2, 30)]);
    }

    #[test]
    #[should_panic(expected = "cannot rewind")]
    fn drain_until_rejects_rewind() {
        let ring = ReorderRing::new(8);
        ring.publish(0, 0u64).unwrap();
        ring.drain_contiguous(|_, _| {});
        // next_deliver = 1, try to drain_until 0 → panic
        ring.drain_until(0, |_, _| {});
    }

    #[test]
    fn drain_until_at_current_is_noop() {
        let ring = ReorderRing::new(8);
        ring.publish(0, 0u64).unwrap();
        ring.drain_contiguous(|_, _| {});
        // next_deliver = 1, drain_until(1) does nothing
        let count = ring.drain_until(1, |_, _| {});
        assert_eq!(count, 0);
    }

    #[test]
    fn concurrent_publish_delivers_in_order() {
        // Issue #6: verify the MP (multi-producer) claim.
        use std::sync::Arc;
        use std::thread;

        let ring = Arc::new(ReorderRing::<u64>::new(64));
        let handles: Vec<_> = (0u64..64).map(|seq| {
            let r = ring.clone();
            thread::spawn(move || {
                r.publish(seq, seq * 10).unwrap();
            })
        }).collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(ring.stored_count(), 64);

        let mut delivered = Vec::new();
        ring.drain_contiguous(|seq, val| delivered.push((seq, val)));
        assert_eq!(delivered.len(), 64);
        for (i, &(seq, val)) in delivered.iter().enumerate() {
            assert_eq!(seq, i as u64);
            assert_eq!(val, i as u64 * 10);
        }
    }

    #[test]
    fn try_deliver_direct_in_order_bypasses_slots() {
        let ring = ReorderRing::<u64>::new(8);

        // seq 0, ring empty — fast path succeeds.
        match ring.try_deliver_direct(0, 100) {
            Ok(val) => assert_eq!(val, 100),
            Err(_) => panic!("fast path should succeed for in-order + empty"),
        }
        assert_eq!(ring.next_deliver(), 1);
        assert_eq!(ring.stored_count(), 0);

        // seq 1, ring empty — fast path succeeds again.
        match ring.try_deliver_direct(1, 200) {
            Ok(val) => assert_eq!(val, 200),
            Err(_) => panic!("fast path should succeed"),
        }
        assert_eq!(ring.next_deliver(), 2);
    }

    #[test]
    fn try_deliver_direct_out_of_order_falls_back() {
        let ring = ReorderRing::<u64>::new(8);

        // seq 1 when next_deliver is 0 — not in order, fast path fails.
        match ring.try_deliver_direct(1, 100) {
            Err(val) => assert_eq!(val, 100),
            Ok(_) => panic!("fast path should fail for out-of-order"),
        }
        assert_eq!(ring.next_deliver(), 0);
    }

    #[test]
    fn try_deliver_direct_fails_when_items_buffered() {
        let ring = ReorderRing::<u64>::new(8);

        // Publish seq 1 (out of order) — now stored_count > 0.
        ring.publish(1, 10).unwrap();
        assert_eq!(ring.stored_count(), 1);

        // seq 0 is in order but ring is not empty — fast path must fail.
        // The consumer must go through publish+drain to ensure the
        // buffered seq 1 is delivered after seq 0.
        match ring.try_deliver_direct(0, 100) {
            Err(val) => assert_eq!(val, 100),
            Ok(_) => panic!("fast path must fail when items are buffered"),
        }
    }

    #[test]
    fn many_cycles_stress() {
        let ring = ReorderRing::<u64>::new(8);
        for cycle in 0..100u64 {
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
        assert_eq!(ring.stored_count(), 0);
    }
}
