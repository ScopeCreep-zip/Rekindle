//! Shared session state — distributed across lane tasks via RwLock.
//!
//! The Control lane task is the exclusive writer. All other lane tasks
//! are readers. RwLock contention is negligible: Control lane writes
//! < 1/sec (key rotation, FIN), duration < 100µs (pointer swap).
//!
//! Cipher keys are Arc-wrapped for lock-free snapshot: readers clone
//! the Arc (one atomic increment) and release the read lock before
//! any AEAD operation. No lock is held during encrypt/decrypt.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::RwLock;

use crate::v4::io::encode::EpochKeys;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::session::state::SessionState;

/// Cipher key set for one epoch. Arc-wrapped so readers can snapshot
/// without holding the RwLock during AEAD operations.
#[derive(Clone)]
pub struct CipherKeySet {
    pub epoch: u8,
    pub keys: EpochKeys,
}

/// Session state shared across all lane tasks.
///
/// # Ownership model
///
/// - **Control lane task:** exclusive writer (via `state.write()`).
///   Writes on key rotation (~1/sec), FIN receipt, capability change.
/// - **All other lane tasks:** readers (via `state.read()`).
///   Read cipher_snapshot for AEAD, check shutting_down flag.
///
/// # Lock discipline
///
/// - Read lock: held for < 100ns (field read or Arc::clone).
///   NEVER held across AEAD seal/open, BLAKE3 hash, or channel send.
/// - Write lock: held for < 100µs (pointer swap for cipher keys).
///   NEVER held across network I/O or allocation.
pub struct SharedSessionState {
    /// Current cipher keys. Swapped atomically on key rotation.
    /// Readers clone the Arc — O(1), no contention with writer.
    pub cipher_keys: Arc<CipherKeySet>,

    /// Session is shutting down. Set by Control lane on FIN receipt.
    /// Checked by Data lane before accepting new streams.
    /// Checked by Handoff lane before accepting new arena setup.
    pub shutting_down: bool,

    /// Current session state (Established, Rotating, Draining, etc).
    pub session_state: SessionState,

    /// Active capabilities negotiated at handshake.
    pub capabilities: CapabilityBits,
}

/// Thread-safe shared state handle. All lane tasks hold a clone.
pub type SessionStateHandle = Arc<RwLock<SharedSessionState>>;

impl SharedSessionState {
    /// Snapshot cipher keys without holding the lock during crypto.
    /// Returns Arc clone — O(1) atomic increment, zero contention.
    #[inline]
    pub fn cipher_snapshot(&self) -> Arc<CipherKeySet> {
        Arc::clone(&self.cipher_keys)
    }

    /// Check if the session is shutting down.
    #[inline]
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }
}

// ── Audit chain link snapshots ──────────────────────────────────
//
// The audit merge task owns both chains. Lane tasks that need
// current_link() for STREAM_ACK, STREAM_FIN, STREAM_CANCEL, and
// AUDIT_CHECKPOINT read from these atomics. The merge task updates
// them on every chain advance. This is the RCU pattern: one writer
// (audit merge), many readers (lane tasks), zero blocking.
//
// Each link is 32 bytes = 4 × u64. We store the 32-byte link as
// four AtomicU64 values. The audit merge task writes all four with
// Release ordering after advancing the chain. Readers load all four
// with Acquire ordering. Torn reads between the four stores are
// acceptable — the link is used for ACK/checkpoint metadata, not
// for cryptographic verification. A torn read produces a stale or
// mixed link value in the ACK payload; the peer re-reads on the
// next checkpoint cycle.

/// Atomic snapshot of a 32-byte audit chain link. Written by the
/// audit merge task, read by lane tasks for STREAM_ACK and
/// AUDIT_CHECKPOINT payloads.
pub struct AtomicChainLink {
    parts: [AtomicU64; 4],
}

impl AtomicChainLink {
    pub fn new(initial: [u8; 32]) -> Self {
        let parts = Self::split(initial);
        Self {
            parts: [
                AtomicU64::new(parts[0]),
                AtomicU64::new(parts[1]),
                AtomicU64::new(parts[2]),
                AtomicU64::new(parts[3]),
            ],
        }
    }

    /// Store a new link value. Called by audit merge after chain.advance().
    pub fn store(&self, link: [u8; 32]) {
        let parts = Self::split(link);
        for (atom, val) in self.parts.iter().zip(parts.iter()) {
            atom.store(*val, Ordering::Release);
        }
    }

    /// Load the current link value. Called by lane tasks for ACK/checkpoint.
    pub fn load(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (i, atom) in self.parts.iter().enumerate() {
            let val = atom.load(Ordering::Acquire);
            bytes[i * 8..(i + 1) * 8].copy_from_slice(&val.to_le_bytes());
        }
        bytes
    }

    fn split(link: [u8; 32]) -> [u64; 4] {
        let mut parts = [0u64; 4];
        for i in 0..4 {
            parts[i] = u64::from_le_bytes(
                link[i * 8..(i + 1) * 8].try_into().unwrap(),
            );
        }
        parts
    }
}

/// Shared audit chain metadata. Published by audit_merge, read by lane tasks.
///
/// Current links, anchors, and lengths are AtomicU64 arrays — zero blocking,
/// zero allocation on the hot path. AUDIT_QUERY proofs use current_link and
/// anchor_link with empty segment — full link history is not published to
/// avoid per-frame cloning overhead at 8×4K120fps throughput.
///
/// `outbound_seq_watch` is a tokio::sync::watch channel that the audit_merge
/// task sends to every time outbound_next_seq advances. The Data lane
/// subscribes to it for FIN readiness re-check. Without this, FIN readiness
/// is only checked when outbound_audit_wake or pending_fin_rx fires, which
/// misses advances that happen after both arms have already returned.
pub struct SharedAuditLinks {
    pub inbound_link: AtomicChainLink,
    pub outbound_link: AtomicChainLink,
    pub inbound_anchor: AtomicChainLink,
    pub outbound_anchor: AtomicChainLink,
    pub inbound_length: AtomicU64,
    pub outbound_length: AtomicU64,
    /// Next expected outbound session_seq — published by audit_merge
    /// after every outbound chain drain. Read by Data lane for FIN
    /// readiness gate: `outbound_next_seq > last_chunk_seq` means
    /// all audit links for the transfer's chunks have been processed.
    pub outbound_next_seq: AtomicU64,
    /// Global inbound progress — published by audit_merge after every
    /// inbound chain drain. The SSOT for "last inbound session_seq
    /// processed across ALL lanes." Read by heartbeat for PING
    /// last_seen_remote_seq. Replaces the per-lane recv_last_seq
    /// shadow that only tracked the Control lane's subset.
    pub recv_last_seq: AtomicU64,
    /// Watch channel for outbound_next_seq advances. audit_merge sends
    /// after every outbound drain. Data lane subscribes for FIN readiness
    /// re-check. The AtomicU64 remains for lock-free reads; the watch
    /// provides the wake signal that the atomic cannot.
    pub outbound_seq_watch_tx: tokio::sync::watch::Sender<u64>,
    pub outbound_seq_watch_rx: tokio::sync::watch::Receiver<u64>,
}

impl SharedAuditLinks {
    pub fn new(initial_link: [u8; 32]) -> Self {
        let (outbound_seq_watch_tx, outbound_seq_watch_rx) = tokio::sync::watch::channel(0u64);
        Self {
            inbound_link: AtomicChainLink::new(initial_link),
            outbound_link: AtomicChainLink::new(initial_link),
            inbound_anchor: AtomicChainLink::new(initial_link),
            outbound_anchor: AtomicChainLink::new(initial_link),
            inbound_length: AtomicU64::new(0),
            outbound_length: AtomicU64::new(0),
            outbound_next_seq: AtomicU64::new(0),
            recv_last_seq: AtomicU64::new(0),
            outbound_seq_watch_tx,
            outbound_seq_watch_rx,
        }
    }

    /// Notify subscribers that outbound_next_seq has advanced.
    /// Called by audit_merge after every outbound chain drain.
    pub fn notify_outbound_advance(&self, next_seq: u64) {
        self.outbound_next_seq.store(next_seq, Ordering::Release);
        let _ = self.outbound_seq_watch_tx.send(next_seq);
    }
}
