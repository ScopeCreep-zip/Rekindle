//! Trait seams — where consumer policy plugs into the buffer mechanism.
//!
//! Three traits, three decisions the crate refuses to make:
//! - [`ReturnGate`]: when a slab is reclaimable (transport completion semantics)
//! - [`SlotLifecycle`]: how a reused slot is reset (secret-hygiene policy)
//! - [`WakeSink`]: how a consumer is woken (runtime choice)
//!
//! Every transport-, crypto-, and runtime-specific concern at the six IPC call
//! sites maps onto exactly one of these three. The buffer crate provides the
//! mechanism; the consumer supplies these three answers.
//!
//! # Shipped implementations
//!
//! Each trait ships with a trivial implementation so consumers that don't need
//! the seam's full generality pay nothing:
//! - [`ImmediateReturn`]: slab is immediately reclaimable (no gate).
//! - [`NoOpLifecycle`]: construct with `Default`, reset is a no-op.
//! - [`SpinWake`]: no-op wake — consumer busy-polls.

// ---------------------------------------------------------------------------
// ReturnGate
// ---------------------------------------------------------------------------

/// Decides when a released slab may return to the free list.
///
/// The pool calls [`is_clear`](ReturnGate::is_clear) during
/// [`SlabPool::reclaim`](crate::SlabPool::reclaim); a slab stays in the
/// pending set until it returns `true`.
///
/// This is the seam that keeps `io_uring` (and RDMA, etc.) out of the buffer
/// crate. The IPC crate implements this with the two-CQE `IORING_CQE_F_NOTIF`
/// gate; D2 implements it with RDMA completion; a trivial consumer uses
/// [`ImmediateReturn`].
///
/// # IPC implementation note
///
/// If `IORING_SEND_ZC_REPORT_USAGE` is set and the send CQE's res has
/// `IORING_NOTIF_USAGE_ZC_COPIED` (bit 31) set, the buffer was copied by the
/// kernel and is immediately safe to reuse. Implementations may short-circuit
/// `is_clear` in this case without waiting for the NOTIF CQE.
///
/// # Provided buffer ring constraint
///
/// Consumers using `io_uring` provided buffer rings (`IORING_REGISTER_PBUF_RING`)
/// must ensure `ring_entries` is a power of two and strictly less than 65536.
/// This is enforced by the kernel at registration time (`kbuf.c:617-621`).
/// When a [`SlabPool`](crate::SlabPool) backs a provided buffer ring, the
/// pool's `capacity` must respect this upper bound.
pub trait ReturnGate: Send + Sync {
    /// Opaque per-slab token the consumer associates with an in-flight
    /// completion.
    ///
    /// - IPC: the `io_uring` `buf_index` / notification `user_data`.
    /// - D2: the RDMA work-request id.
    /// - Trivial: `()`.
    type Token: Copy + Send + Sync;

    /// `true` iff the slab tagged with `token` is safe to reclaim.
    fn is_clear(&self, token: Self::Token) -> bool;

    /// Called when the pool hands a slab to the gate on
    /// [`SlabPool::release`](crate::SlabPool::release). The gate records the
    /// slab's index and returns a token for later `is_clear` checks.
    fn on_release(&self, index: usize) -> Self::Token;
}

/// Trivial [`ReturnGate`]: the slab is immediately reclaimable. Use this for
/// consumers where a released slab has no in-flight transport operation to
/// await (in-process fabric, synchronous tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct ImmediateReturn;

impl ReturnGate for ImmediateReturn {
    type Token = ();

    #[inline]
    fn is_clear(&self, _token: ()) -> bool {
        true
    }

    #[inline]
    fn on_release(&self, _index: usize) {}
}

// ---------------------------------------------------------------------------
// SlotLifecycle
// ---------------------------------------------------------------------------

/// Owns the construct-once / reset-on-reuse discipline for a reusable slot.
///
/// # Contract
///
/// - [`construct`](SlotLifecycle::construct) is called once per slot at pool
///   or ring construction time. It may allocate.
/// - [`reset`](SlotLifecycle::reset) is called on every reuse (when a slab
///   returns to the free list). It **must not allocate** — this is the
///   invariant that keeps the steady state allocation-free.
///
/// # Where the zeroize hook lives
///
/// The IPC crate's recv-side `SlotLifecycle::reset` calls `zeroize` on the
/// plaintext buffer (secret hygiene). The send-side `reset` clears the
/// buffer's length and keeps capacity. A non-secret consumer's `reset` is a
/// cheap truncate or no-op.
pub trait SlotLifecycle<T>: Send + Sync {
    /// Construct one slot's payload. Called `capacity` times at construction —
    /// this is the single allocation point.
    fn construct(&self) -> T;

    /// Reset a slot for reuse. **Must not allocate.** Called on every return
    /// to the free list.
    fn reset(&self, slot: &mut T);
}

/// Trivial [`SlotLifecycle`]: construct via [`Default::default()`], reset is a
/// no-op. Use this for payloads that don't need cleanup between uses.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpLifecycle;

impl<T: Default + Send + Sync> SlotLifecycle<T> for NoOpLifecycle {
    #[inline]
    fn construct(&self) -> T {
        T::default()
    }

    #[inline]
    fn reset(&self, _slot: &mut T) {}
}

// ---------------------------------------------------------------------------
// WakeSink
// ---------------------------------------------------------------------------

/// Non-blocking consumer wake signal.
///
/// Producers call [`wake`](WakeSink::wake) after making progress (a slot
/// filled, a slab freed, credit released). The consumer receives the signal
/// and polls for work.
///
/// The core crate is runtime-free; this trait is how a runtime plugs in.
/// The buffer primitives themselves do **not** call `wake` — they return to
/// the caller, and the caller decides whether to wake. This keeps the core
/// completely decoupled from any runtime.
///
/// # Implementations
///
/// - [`SpinWake`]: no-op — consumer busy-polls. Shipped in the crate.
/// - `TokioWake`: `Arc<tokio::sync::Notify>` — behind feature `"tokio"`.
/// - `ParkerWake`: `crossbeam::sync::Unparker` — the blocking non-tokio path.
///
/// # Semantics
///
/// - `wake()` is idempotent and non-blocking.
/// - Multiple wakes before one consumer poll collapse to one (level-triggered).
/// - `wake()` must never block the calling thread.
pub trait WakeSink: Clone + Send + Sync + 'static {
    /// Signal the consumer that progress was made.
    fn wake(&self);
}

/// No-op [`WakeSink`]: the consumer busy-polls. Zero overhead.
///
/// Use this for benchmarks, tests, or consumers on a dedicated thread that
/// spin-polls the ring/queue in a tight loop.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpinWake;

impl WakeSink for SpinWake {
    #[inline]
    fn wake(&self) {}
}
