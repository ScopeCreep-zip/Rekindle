//! Bulk transfer and heartbeat observability counters.
//!
//! One `Arc<BulkCounters>` per process, shared across all connections.
//! Incremented at the write task (sent), read task (received), and
//! heartbeat state machine (pong events).
//! Read by status endpoints, Prometheus, TUI.
//!
//! Each field is wrapped in `CachePadded<AtomicU64>` to prevent false
//! sharing between fields written by different threads. On x86-64,
//! CachePadded aligns and pads to 128 bytes (Intel spatial prefetcher
//! pulls 128-byte pairs). Without padding, a rayon worker incrementing
//! `frames_sent` invalidates the cache line containing `pool_acquires`
//! on the pool thread. At 200K connections × 64 chunks/transfer, this
//! causes millions of false-sharing invalidations per bulk batch.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_utils::CachePadded;

// Global leak-diagnosis counters. Zero-cost fetch_add(Relaxed).
// Accessible from any thread without passing references.
pub static DIAG_SEND_PLAINTEXT_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_WIRE_POOL_HITS: AtomicU64 = AtomicU64::new(0);
pub static DIAG_WIRE_POOL_MISSES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_WIRE_POOL_RETURNS: AtomicU64 = AtomicU64::new(0);
pub static DIAG_WIRE_POOL_OVERFLOW_DROPS: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_PLAINTEXT_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RETENTION_STORED_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RETENTION_STORED_FRAMES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_DELIVERED_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_DELIVERED_FRAMES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_POOL_ACQUIRES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_POOL_REUSED: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_POOL_FRESH_ALLOC: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_POOL_RELEASED: AtomicU64 = AtomicU64::new(0);
pub static DIAG_RECV_POOL_OVERFLOW_DROPPED: AtomicU64 = AtomicU64::new(0);
// Audit DispatchQueue diagnostics
pub static DIAG_AUDIT_OUTBOUND_PUSHES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_AUDIT_OUTBOUND_PUSH_FAILURES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_AUDIT_INBOUND_PUSHES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_AUDIT_INBOUND_PUSH_FAILURES: AtomicU64 = AtomicU64::new(0);
pub static DIAG_AUDIT_OUTBOUND_POPS: AtomicU64 = AtomicU64::new(0);
pub static DIAG_AUDIT_INBOUND_POPS: AtomicU64 = AtomicU64::new(0);

/// Reset all global diagnostic counters to zero. Called at the start of
/// pressure tests to isolate measurements from prior tests in the same
/// process. Uses Relaxed ordering — counters are diagnostic, not synchronization.
pub fn reset_all_diagnostics() {
    DIAG_SEND_PLAINTEXT_BYTES.store(0, Ordering::Relaxed);
    DIAG_WIRE_POOL_HITS.store(0, Ordering::Relaxed);
    DIAG_WIRE_POOL_MISSES.store(0, Ordering::Relaxed);
    DIAG_WIRE_POOL_RETURNS.store(0, Ordering::Relaxed);
    DIAG_WIRE_POOL_OVERFLOW_DROPS.store(0, Ordering::Relaxed);
    DIAG_RECV_PLAINTEXT_BYTES.store(0, Ordering::Relaxed);
    DIAG_RETENTION_STORED_BYTES.store(0, Ordering::Relaxed);
    DIAG_RETENTION_STORED_FRAMES.store(0, Ordering::Relaxed);
    DIAG_RECV_DELIVERED_BYTES.store(0, Ordering::Relaxed);
    DIAG_RECV_DELIVERED_FRAMES.store(0, Ordering::Relaxed);
    DIAG_RECV_POOL_ACQUIRES.store(0, Ordering::Relaxed);
    DIAG_RECV_POOL_REUSED.store(0, Ordering::Relaxed);
    DIAG_RECV_POOL_FRESH_ALLOC.store(0, Ordering::Relaxed);
    DIAG_RECV_POOL_RELEASED.store(0, Ordering::Relaxed);
    DIAG_RECV_POOL_OVERFLOW_DROPPED.store(0, Ordering::Relaxed);
    DIAG_AUDIT_OUTBOUND_PUSHES.store(0, Ordering::Relaxed);
    DIAG_AUDIT_OUTBOUND_PUSH_FAILURES.store(0, Ordering::Relaxed);
    DIAG_AUDIT_INBOUND_PUSHES.store(0, Ordering::Relaxed);
    DIAG_AUDIT_INBOUND_PUSH_FAILURES.store(0, Ordering::Relaxed);
    DIAG_AUDIT_OUTBOUND_POPS.store(0, Ordering::Relaxed);
    DIAG_AUDIT_INBOUND_POPS.store(0, Ordering::Relaxed);
}

pub struct BulkCounters {
    // Write task
    pub frames_sent: CachePadded<AtomicU64>,
    pub bytes_sent: CachePadded<AtomicU64>,
    // Read task
    pub frames_received: CachePadded<AtomicU64>,
    pub bytes_received: CachePadded<AtomicU64>,
    // Pool thread
    pub pool_acquires: CachePadded<AtomicU64>,
    pub pool_contention_parks: CachePadded<AtomicU64>,
    // Read task
    pub replay_rejections: CachePadded<AtomicU64>,
    pub aead_failures: CachePadded<AtomicU64>,
    // Control loop
    /// PONG received with nonce matching a previous (timed-out) PING.
    /// Not fatal — the peer responded but the response arrived late.
    /// High counts indicate the control loop is under sustained load.
    pub heartbeat_stale_pongs: CachePadded<AtomicU64>,
    /// PONG received with no PING outstanding at all (unsolicited).
    /// May indicate a protocol violation or a very late response after
    /// the nonce was already cleared by a subsequent successful PONG.
    pub heartbeat_unsolicited_pongs: CachePadded<AtomicU64>,
    /// PONG received with nonce matching the current outstanding PING.
    /// Normal operation counter.
    pub heartbeat_pongs_accepted: CachePadded<AtomicU64>,
    /// Pong timeout fired — the peer did not respond within pong_timeout.
    pub heartbeat_pong_timeouts: CachePadded<AtomicU64>,
    // CreditGuard admission
    /// Inbound bulk frames shed because CreditGuard::try_reserve returned false.
    pub memory_pressure_drops: CachePadded<AtomicU64>,
    // Pipeline allocation tracking (zero-cost atomic counters)
    /// Bytes allocated by chunk.to_vec() in BulkSender (send plaintext copy)
    pub send_plaintext_alloc_bytes: CachePadded<AtomicU64>,
    /// WireBufPool hits (reused from freelist)
    pub wire_pool_hits: CachePadded<AtomicU64>,
    /// WireBufPool misses (fresh Vec allocation)
    pub wire_pool_misses: CachePadded<AtomicU64>,
    /// Bytes allocated by cipher.open() in BulkReceiver (recv plaintext)
    pub recv_plaintext_alloc_bytes: CachePadded<AtomicU64>,
    /// Bytes stored in retention buffer
    pub retention_stored_bytes: CachePadded<AtomicU64>,
    /// Frames stored in retention buffer
    pub retention_stored_frames: CachePadded<AtomicU64>,
    /// Bytes delivered through bulk_data_tx to application
    pub recv_delivered_bytes: CachePadded<AtomicU64>,
}

static_assertions::assert_impl_all!(BulkCounters: Send, Sync);

impl BulkCounters {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            frames_sent: CachePadded::new(AtomicU64::new(0)),
            frames_received: CachePadded::new(AtomicU64::new(0)),
            bytes_sent: CachePadded::new(AtomicU64::new(0)),
            bytes_received: CachePadded::new(AtomicU64::new(0)),
            pool_acquires: CachePadded::new(AtomicU64::new(0)),
            pool_contention_parks: CachePadded::new(AtomicU64::new(0)),
            replay_rejections: CachePadded::new(AtomicU64::new(0)),
            aead_failures: CachePadded::new(AtomicU64::new(0)),
            heartbeat_stale_pongs: CachePadded::new(AtomicU64::new(0)),
            heartbeat_unsolicited_pongs: CachePadded::new(AtomicU64::new(0)),
            heartbeat_pongs_accepted: CachePadded::new(AtomicU64::new(0)),
            heartbeat_pong_timeouts: CachePadded::new(AtomicU64::new(0)),
            memory_pressure_drops: CachePadded::new(AtomicU64::new(0)),
            send_plaintext_alloc_bytes: CachePadded::new(AtomicU64::new(0)),
            wire_pool_hits: CachePadded::new(AtomicU64::new(0)),
            wire_pool_misses: CachePadded::new(AtomicU64::new(0)),
            recv_plaintext_alloc_bytes: CachePadded::new(AtomicU64::new(0)),
            retention_stored_bytes: CachePadded::new(AtomicU64::new(0)),
            retention_stored_frames: CachePadded::new(AtomicU64::new(0)),
            recv_delivered_bytes: CachePadded::new(AtomicU64::new(0)),
        })
    }

    pub fn record_sent(&self, frames: u64, bytes: u64) {
        self.frames_sent.fetch_add(frames, Ordering::Relaxed);
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_received(&self, frames: u64, bytes: u64) {
        self.frames_received.fetch_add(frames, Ordering::Relaxed);
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            frames_sent: self.frames_sent.load(Ordering::Relaxed),
            frames_received: self.frames_received.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            pool_acquires: self.pool_acquires.load(Ordering::Relaxed),
            pool_contention_parks: self.pool_contention_parks.load(Ordering::Relaxed),
            replay_rejections: self.replay_rejections.load(Ordering::Relaxed),
            aead_failures: self.aead_failures.load(Ordering::Relaxed),
            heartbeat_stale_pongs: self.heartbeat_stale_pongs.load(Ordering::Relaxed),
            heartbeat_unsolicited_pongs: self.heartbeat_unsolicited_pongs.load(Ordering::Relaxed),
            heartbeat_pongs_accepted: self.heartbeat_pongs_accepted.load(Ordering::Relaxed),
            heartbeat_pong_timeouts: self.heartbeat_pong_timeouts.load(Ordering::Relaxed),
            memory_pressure_drops: self.memory_pressure_drops.load(Ordering::Relaxed),
            send_plaintext_alloc_bytes: self.send_plaintext_alloc_bytes.load(Ordering::Relaxed),
            wire_pool_hits: self.wire_pool_hits.load(Ordering::Relaxed),
            wire_pool_misses: self.wire_pool_misses.load(Ordering::Relaxed),
            recv_plaintext_alloc_bytes: self.recv_plaintext_alloc_bytes.load(Ordering::Relaxed),
            retention_stored_bytes: self.retention_stored_bytes.load(Ordering::Relaxed),
            retention_stored_frames: self.retention_stored_frames.load(Ordering::Relaxed),
            recv_delivered_bytes: self.recv_delivered_bytes.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CounterSnapshot {
    pub frames_sent: u64,
    pub frames_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub pool_acquires: u64,
    pub pool_contention_parks: u64,
    pub replay_rejections: u64,
    pub aead_failures: u64,
    pub heartbeat_stale_pongs: u64,
    pub heartbeat_unsolicited_pongs: u64,
    pub heartbeat_pongs_accepted: u64,
    pub heartbeat_pong_timeouts: u64,
    pub memory_pressure_drops: u64,
    pub send_plaintext_alloc_bytes: u64,
    pub wire_pool_hits: u64,
    pub wire_pool_misses: u64,
    pub recv_plaintext_alloc_bytes: u64,
    pub retention_stored_bytes: u64,
    pub retention_stored_frames: u64,
    pub recv_delivered_bytes: u64,
}
