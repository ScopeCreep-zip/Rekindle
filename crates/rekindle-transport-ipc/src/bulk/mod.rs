//! Bulk transfer plane: parallel AES-256-GCM encryption/decryption.
//!
//! The control plane (Noise IK) is optimized for small messages at low
//! latency. The bulk plane uses aws-lc-rs LessSafeKey with explicit
//! nonces, enabling parallel encryption across rayon workers.
//!
//! Both planes derive keys from the same Noise handshake hash via
//! HKDF-SHA-256 with distinct domain-specific labels.
//!
//! All plaintext is zeroized after encryption — no exceptions.

pub mod cipher;
pub mod kdf;
pub mod nonce;
pub mod frame;
pub mod pool;
pub mod encrypt;
pub mod stream;
pub mod dispatcher;
pub mod replay;
pub mod reassembly;
pub mod verify;
pub mod transfer;
// ---- Public API re-exports ----

pub use cipher::BulkCipher;
pub use nonce::NonceCounter;
pub use pool::{BufferPool, PooledBuf, ZeroizingBuf};
pub use encrypt::build_encrypt_pool;
pub use stream::{BulkStream, BulkStreamId};
pub use stream::ZeroizingStream;
pub use dispatcher::{BulkDispatcher, DecryptedChunk, DEFAULT_REASSEMBLY_CAPACITY};
pub use replay::ReplayFilter;
pub use reassembly::{Reassembler, ReassembledChunk};
pub use verify::{DigestAlgorithm, MerkleDigest, StreamingDigest};
pub use kdf::BulkKeyPair;
pub use frame::{BulkFrameHeader, FrameKind, HEADER_LEN, TAG_LEN, MAX_CHUNK_PLAIN};

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

/// Shared atomic counters for bulk transfer observability.
///
/// Covers all 6 pipeline stages:
/// 1. frames_sent / bytes_sent — written to socket by write task
/// 2. frames_received / bytes_received — read from socket by read task
/// 3. chunks_decrypted — completed rayon decrypt tasks
/// 4. chunks_reassembled — delivered in-order by reassembler
/// 5. transfers_completed — full payloads delivered to FrameRouter
pub struct BulkCounters {
    pub frames_sent: AtomicU64,
    pub frames_received: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub bytes_received: AtomicU64,
    pub chunks_decrypted: AtomicU64,
    pub chunks_reassembled: AtomicU64,
    pub transfers_completed: AtomicU64,
}

static_assertions::assert_impl_all!(BulkCounters: Send, Sync);

impl BulkCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            frames_sent: AtomicU64::new(0),
            frames_received: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            chunks_decrypted: AtomicU64::new(0),
            chunks_reassembled: AtomicU64::new(0),
            transfers_completed: AtomicU64::new(0),
        })
    }
}
