//! Priority-separated channels for wire bytes to the write task.
//!
//! Five channels: Control > Audit > Handoff > Data (lifecycle) > Bulk (payload).
//!
//! All five channels are crossbeam bounded. `send()` is sync `try_send()` —
//! non-blocking, safe from async context on every platform. The callers
//! (control loop, drain_outbound) never `.await` on lane sends. The
//! `SendError` enum is the stable contract — callers match on `ChannelFull`
//! vs `ChannelClosed` and never know the underlying channel implementation.
//!
//! On Linux (io_uring): the uring write task consumes crossbeam receivers
//! directly via `Select::new()`. Zero bridge threads on the write path.
//!
//! The bulk channel carries `BulkFrame` — either a pooled `WireBuf` (payload
//! chunks, self-returning to freelist on Drop) or a plain `Vec<u8>` (FIN and
//! other lifecycle frames). The write task Derefs to `&[u8]` regardless of
//! variant. Small-frame channels carry `Vec<u8>`.
//!
//! Shutdown sequence:
//! 1. Control loop exits → `LaneChannels` dropped → crossbeam senders dropped
//! 2. Write task's crossbeam `recv()` returns `Err(Disconnected)`
//! 3. Write task exits → socket closed → peer gets EOF

use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossbeam::queue::ArrayQueue;

use crate::v4::bulk::counters;
use crate::v4::wire::lane::Lane;

// ---------------------------------------------------------------------------
// WireBufPool + WireBuf — self-returning wire buffer freelist
// ---------------------------------------------------------------------------

/// Lock-free freelist of reusable wire buffers.
/// Shared between BulkSender (acquire) and the write task (Drop returns).
#[derive(Clone)]
pub struct WireBufPool {
    free: Arc<ArrayQueue<Vec<u8>>>,
}

impl WireBufPool {
    /// Create a freelist with `count` slots. Buffers are allocated on demand
    /// at `acquire(capacity)` time and returned to the pool on Drop.
    ///
    /// No pre-allocation: the FIFO ArrayQueue rotation problem faults all
    /// pre-allocated pages even when only 1-2 buffers are concurrently needed.
    /// On-demand allocation converges to steady-state within the first few
    /// transfers — acceptable for a global pool that lives for the process
    /// lifetime.
    pub fn new(count: usize) -> Self {
        let free = Arc::new(ArrayQueue::new(count.max(1)));
        Self { free }
    }

    /// Acquire and wrap in a WireBuf that returns to this pool on Drop.
    pub fn acquire(&self, capacity: usize) -> WireBuf {
        let inner = match self.free.pop() {
            Some(mut buf) => {
                buf.clear();
                if buf.capacity() < capacity {
                    // After clear(), len = 0. reserve(n) guarantees
                    // capacity >= len + n = 0 + n = n. Pass the full
                    // required capacity, NOT the delta from old capacity.
                    buf.reserve(capacity);
                }
                counters::DIAG_WIRE_POOL_HITS.fetch_add(1, Ordering::Relaxed);
                buf
            }
            None => {
                counters::DIAG_WIRE_POOL_MISSES.fetch_add(1, Ordering::Relaxed);
                Vec::with_capacity(capacity)
            }
        };
        WireBuf {
            inner: Some(inner),
            pool: self.clone(),
        }
    }

    fn release(&self, buf: Vec<u8>) {
        if self.free.push(buf).is_ok() {
            counters::DIAG_WIRE_POOL_RETURNS.fetch_add(1, Ordering::Relaxed);
        } else {
            counters::DIAG_WIRE_POOL_OVERFLOW_DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A wire buffer that returns its Vec<u8> to the freelist on Drop.
/// The write task receives this inside a `BulkFrame::Pooled`, writes
/// `&self[..]` to the socket, and drops it — the Drop impl returns
/// the Vec to the pool. No return channel. No coupling.
pub struct WireBuf {
    inner: Option<Vec<u8>>,
    pool: WireBufPool,
}

impl WireBuf {
    /// Mutable access to the inner Vec for filling.
    pub fn inner_mut(&mut self) -> &mut Vec<u8> {
        self.inner.as_mut().expect("WireBuf used after take")
    }
}

impl Deref for WireBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.inner.as_ref().expect("WireBuf used after take")
    }
}

impl AsRef<[u8]> for WireBuf {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_ref().expect("WireBuf used after take")
    }
}

impl Drop for WireBuf {
    fn drop(&mut self) {
        if let Some(buf) = self.inner.take() {
            self.pool.release(buf);
        }
    }
}

// WireBuf is Send — Vec<u8> is Send, WireBufPool (Arc<ArrayQueue>) is Send.
static_assertions::assert_impl_all!(WireBuf: Send);

// ---------------------------------------------------------------------------
// RecvBufPool + RecvBuf — self-returning plaintext buffer freelist (recv side)
// ---------------------------------------------------------------------------

/// Lock-free freelist of reusable plaintext buffers for the recv pipeline.
/// Same architecture as WireBufPool. Pre-allocated buffers are acquired by
/// the dispatching thread, filled by the rayon worker via cipher.open_into(),
/// and returned to the pool when the application drops the delivered chunk.
/// Zero allocations on rayon worker threads.
#[derive(Clone)]
pub struct RecvBufPool {
    free: Arc<ArrayQueue<Vec<u8>>>,
}

impl RecvBufPool {
    /// Create a freelist with capacity for `count` buffers.
    /// Buffers are NOT pre-allocated — they are allocated on first acquire
    /// and returned to the pool on drop. This prevents faulting pages for
    /// all buffers when only 1-2 are needed in steady state.
    pub fn new(count: usize) -> Self {
        let free = Arc::new(ArrayQueue::new(count.max(1)));
        Self { free }
    }

    /// Acquire a buffer, clearing it for reuse. Falls back to fresh allocation
    /// if the freelist is empty.
    pub fn acquire(&self, capacity: usize) -> RecvBuf {
        counters::DIAG_RECV_POOL_ACQUIRES.fetch_add(1, Ordering::Relaxed);
        let inner = match self.free.pop() {
            Some(mut buf) => {
                counters::DIAG_RECV_POOL_REUSED.fetch_add(1, Ordering::Relaxed);
                buf.clear();
                if buf.capacity() < capacity {
                    // After clear(), len = 0. reserve(n) guarantees
                    // capacity >= len + n = 0 + n = n. Pass the full
                    // required capacity, NOT the delta from old capacity.
                    buf.reserve(capacity);
                }
                buf
            }
            None => {
                counters::DIAG_RECV_POOL_FRESH_ALLOC.fetch_add(1, Ordering::Relaxed);
                Vec::with_capacity(capacity)
            }
        };
        RecvBuf {
            inner: Some(inner),
            len: 0,
            pool: self.clone(),
        }
    }

    fn release(&self, buf: Vec<u8>) {
        if self.free.push(buf).is_err() {
            counters::DIAG_RECV_POOL_OVERFLOW_DROPPED.fetch_add(1, Ordering::Relaxed);
        } else {
            counters::DIAG_RECV_POOL_RELEASED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A recv plaintext buffer that returns its Vec<u8> to the freelist on Drop.
/// Carries a logical length (set after cipher.open_into fills it).
pub struct RecvBuf {
    inner: Option<Vec<u8>>,
    len: usize,
    pool: RecvBufPool,
}

impl RecvBuf {
    /// Mutable access to the full capacity of the backing buffer for
    /// cipher.open_into(). Extends the Vec's logical length to cover the
    /// entire allocation WITHOUT zeroing. The cipher writes all bytes
    /// before any read occurs. The caller MUST call set_len() with the
    /// actual written length after filling.
    ///
    /// # Safety (internal)
    ///
    /// Uses `set_len(cap)` without initialization. This is sound because:
    /// - The cipher's `open_into()` writes `plaintext_len` bytes before returning
    /// - `RecvBuf::set_len(plaintext_len)` is called immediately after, constraining
    ///   the readable view to only the bytes the cipher wrote
    /// - No code reads beyond `self.len` (enforced by `as_bytes()` and `Deref`)
    #[allow(unsafe_code)]
    pub fn buf_mut(&mut self) -> &mut [u8] {
        let v = self.inner.as_mut().expect("RecvBuf used after take");
        let cap = v.capacity();
        // SAFETY: cipher.open_into() writes all bytes in [0..plaintext_len]
        // before returning. set_len(plaintext_len) is called by the caller
        // immediately after. No uninitialized bytes are ever read.
        unsafe { v.set_len(cap) };
        v.as_mut_slice()
    }

    /// Set the logical length after decryption. Must be ≤ capacity.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
    }

    /// The decrypted plaintext bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner.as_ref().expect("RecvBuf used after take")[..self.len]
    }

    /// Copy the plaintext bytes into a new owned Vec<u8>.
    /// The RecvBuf continues to own its pooled buffer — it returns to the
    /// pool on Drop. Use this when ownership of the bytes is required
    /// without draining the pool.
    pub fn to_owned_vec(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl Deref for RecvBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Drop for RecvBuf {
    fn drop(&mut self) {
        if let Some(buf) = self.inner.take() {
            self.pool.release(buf);
        }
    }
}

static_assertions::assert_impl_all!(RecvBuf: Send);

// ---------------------------------------------------------------------------
// PlaintextBuf — the pipeline buffer type for decrypted chunk data
// ---------------------------------------------------------------------------

/// A decrypted plaintext buffer. Either pooled (from bulk decrypt, returns
/// to RecvBufPool on Drop) or owned (from inline decrypt, dropped normally).
///
/// The reassembler, delivery pipeline, and application all see `&[u8]`
/// via Deref. When the application drops the chunk, pooled buffers cycle
/// back to the freelist; owned buffers are freed by glibc.
pub enum PlaintextBuf {
    /// From the bulk decrypt pipeline — returns to RecvBufPool on Drop.
    Pooled(RecvBuf),
    /// From the inline decrypt path — dropped normally.
    Owned(Vec<u8>),
}

impl Deref for PlaintextBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Pooled(rb) => rb.as_bytes(),
            Self::Owned(v) => v,
        }
    }
}

impl AsRef<[u8]> for PlaintextBuf {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Pooled(rb) => rb.as_bytes(),
            Self::Owned(v) => v,
        }
    }
}

impl PlaintextBuf {
    /// Length of the plaintext.
    pub fn len(&self) -> usize {
        match self {
            Self::Pooled(rb) => rb.as_bytes().len(),
            Self::Owned(v) => v.len(),
        }
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static_assertions::assert_impl_all!(PlaintextBuf: Send);

// ---------------------------------------------------------------------------
// BulkFrame — the bulk channel payload type
// ---------------------------------------------------------------------------

/// A frame on the bulk wire channel. Either a pooled wire buffer (payload
/// chunks from BulkSender) or a plain Vec<u8> (FIN and other lifecycle
/// frames). The write task Derefs to `&[u8]` regardless of variant.
pub enum BulkFrame {
    /// Payload chunk — self-returning to the wire buffer freelist on Drop.
    Pooled(WireBuf),
    /// Lifecycle frame (FIN, CANCEL) — plain bytes, dropped normally.
    Plain(Vec<u8>),
}

impl Deref for BulkFrame {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Pooled(wb) => wb,
            Self::Plain(v) => v,
        }
    }
}

impl AsRef<[u8]> for BulkFrame {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Pooled(wb) => wb.as_ref(),
            Self::Plain(v) => v,
        }
    }
}

static_assertions::assert_impl_all!(BulkFrame: Send);

// ---------------------------------------------------------------------------
// SendError
// ---------------------------------------------------------------------------

/// Errors from lane channel send operations.
#[derive(Debug)]
pub enum SendError {
    /// The write task is alive but behind — channel at capacity.
    ChannelFull,
    /// The write task has exited — all receivers dropped.
    ChannelClosed,
}

impl<T> From<crossbeam::channel::TrySendError<T>> for SendError {
    fn from(e: crossbeam::channel::TrySendError<T>) -> Self {
        match e {
            crossbeam::channel::TrySendError::Full(_) => Self::ChannelFull,
            crossbeam::channel::TrySendError::Disconnected(_) => Self::ChannelClosed,
        }
    }
}

impl<T> From<crossbeam::channel::SendError<T>> for SendError {
    fn from(_: crossbeam::channel::SendError<T>) -> Self {
        Self::ChannelClosed
    }
}

// ---------------------------------------------------------------------------
// LaneChannels / BulkWireSender / LaneReceivers
// ---------------------------------------------------------------------------

/// Small-frame sender — cloneable for distribution to lane tasks.
///
/// Each lane task, audit merge task, and drain pipeline needs to send
/// outbound wire bytes to the write task. crossbeam::channel::Sender is
/// Clone (Arc-based internally), so cloning LaneChannels is 4 atomic
/// increments — no allocation, no contention.
#[derive(Clone)]
pub struct LaneChannels {
    control_tx: crossbeam::channel::Sender<Vec<u8>>,
    audit_tx: crossbeam::channel::Sender<Vec<u8>>,
    handoff_tx: crossbeam::channel::Sender<Vec<u8>>,
    data_tx: crossbeam::channel::Sender<Vec<u8>>,
}

/// Bulk-frame sender — cloned into rayon workers via BulkSender.
#[derive(Clone)]
pub struct BulkWireSender {
    crossbeam_tx: crossbeam::channel::Sender<BulkFrame>,
}

impl BulkWireSender {
    /// Non-blocking send. Returns immediately on Full or Closed.
    /// Rayon workers MUST NOT block — a blocked worker starves the pool.
    /// The inflight_sem ensures at most `rayon_workers × 2` items are
    /// in-flight simultaneously, so the channel should never be full
    /// unless the write task is dead.
    pub fn send(&self, frame: BulkFrame) -> Result<(), SendError> {
        self.crossbeam_tx
            .try_send(frame)
            .map_err(Into::into)
    }

    /// Extract the underlying crossbeam sender.
    pub fn into_crossbeam_sender(self) -> crossbeam::channel::Sender<BulkFrame> {
        self.crossbeam_tx
    }
}

/// Receiver side — owned by the write task.
pub struct LaneReceivers {
    pub control_rx: crossbeam::channel::Receiver<Vec<u8>>,
    pub audit_rx: crossbeam::channel::Receiver<Vec<u8>>,
    pub handoff_rx: crossbeam::channel::Receiver<Vec<u8>>,
    pub data_rx: crossbeam::channel::Receiver<Vec<u8>>,
    pub bulk_rx: crossbeam::channel::Receiver<BulkFrame>,
}


impl LaneChannels {
    /// Create all five channels as crossbeam bounded.
    pub fn new(capacity: usize) -> (Self, BulkWireSender, LaneReceivers) {
        let (control_tx, control_rx) = crossbeam::channel::bounded(capacity);
        let (audit_tx, audit_rx) = crossbeam::channel::bounded(capacity);
        let (handoff_tx, handoff_rx) = crossbeam::channel::bounded(capacity);
        let (data_tx, data_rx) = crossbeam::channel::bounded(capacity);
        let (bulk_tx, bulk_rx) = crossbeam::channel::bounded(capacity);

        (
            Self { control_tx, audit_tx, handoff_tx, data_tx },
            BulkWireSender { crossbeam_tx: bulk_tx },
            LaneReceivers { control_rx, audit_rx, handoff_rx, data_rx, bulk_rx },
        )
    }

    /// Send pre-encoded wire bytes to the correct lane channel.
    pub fn send(&self, lane: Lane, wire_bytes: Vec<u8>) -> Result<(), SendError> {
        match lane {
            Lane::Control => self.control_tx.try_send(wire_bytes)?,
            Lane::Audit => self.audit_tx.try_send(wire_bytes)?,
            Lane::Handoff => self.handoff_tx.try_send(wire_bytes)?,
            Lane::Data => self.data_tx.try_send(wire_bytes)?,
        }
        Ok(())
    }

    /// Send with short async backpressure.
    ///
    /// Retries up to 5 times with exponential backoff (10µs → 160µs, ~310µs total).
    /// If the write task cannot drain one slot in ~310µs, it is dead or fatally
    /// behind — return ChannelFull and let the control loop terminate the session.
    pub async fn send_with_retry(
        &self,
        lane: Lane,
        wire_bytes: Vec<u8>,
    ) -> Result<(), SendError> {
        let tx = match lane {
            Lane::Control => &self.control_tx,
            Lane::Audit => &self.audit_tx,
            Lane::Handoff => &self.handoff_tx,
            Lane::Data => &self.data_tx,
        };

        let mut bytes = wire_bytes;
        match tx.try_send(bytes) {
            Ok(()) => return Ok(()),
            Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
                return Err(SendError::ChannelClosed);
            }
            Err(crossbeam::channel::TrySendError::Full(returned)) => {
                bytes = returned;
            }
        }

        let mut backoff_us = 10u64;
        for _ in 0..5 {
            tokio::time::sleep(std::time::Duration::from_micros(backoff_us)).await;
            match tx.try_send(bytes) {
                Ok(()) => return Ok(()),
                Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
                    return Err(SendError::ChannelClosed);
                }
                Err(crossbeam::channel::TrySendError::Full(returned)) => {
                    bytes = returned;
                    backoff_us *= 2;
                }
            }
        }

        tracing::debug!("lane channel stalled — write task did not drain after 5 retries (~310µs)");
        Err(SendError::ChannelFull)
    }
}
