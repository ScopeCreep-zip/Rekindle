//! Bulk-transfer plane for the Rekindle IPC bus.
//!
//! The control plane (request/response/event) uses Noise IK encryption
//! with `TransportState` (&mut self, auto-nonce, ~47ns routing). It is
//! optimized for small messages (49–300 bytes) at low latency.
//!
//! The bulk plane uses aws-lc-rs `LessSafeKey` AES-256-GCM with explicit
//! nonces, enabling parallel encryption across rayon workers. It is
//! optimized for large payloads (64 KiB chunks) at high throughput.
//!
//! # Quick start
//!
//! ```ignore
//! use std::sync::Arc;
//! use rekindle_node::ipc::bulk::*;
//!
//! // 1. Build the encrypt pool (once at startup).
//! let encrypt_pool = build_encrypt_pool();
//!
//! // 2. Create the buffer pool (once at startup).
//! let buffer_pool = BufferPool::new();
//!
//! // 3. Derive cipher from a Noise handshake hash.
//! let cipher = Arc::new(kdf::derive_bulk_cipher(&handshake_hash));
//!
//! // 4. Create a zeroizing bulk stream for sending.
//! let (tx, rx) = crossbeam::channel::bounded(64);
//! let stream = ZeroizingStream::new(
//!     0,
//!     cipher,
//!     Arc::new(NonceCounter::new()),
//!     buffer_pool,
//!     tx,
//! );
//!
//! // 5. Submit chunks for parallel encryption (plaintext is zeroized after encrypt).
//! stream.submit_chunk(&encrypt_pool, &chunk_bytes, false);
//!
//! // 6. Drain encrypted frames from rx and write to socket.
//! ```
//!
//! Both planes derive their keys from the same Noise session handshake
//! hash via HKDF-SHA-256 with distinct domain-specific labels.
//!
//! # Data flow (send side)
//!
//! ```text
//! Blob source (file, registry, containerd)
//!   │
//!   ├── chunk into ≤65,519-byte segments
//!   │
//!   ├── ZeroizingStream::submit_chunk() [per chunk]
//!   │   ├── NonceCounter::next() → nonce (aborts process on exhaustion)
//!   │   └── rayon::spawn(move || {
//!   │       ├── BufferPool::acquire() → slab
//!   │       ├── write header into slab
//!   │       ├── copy plaintext into slab (Zeroizing<Vec<u8>>)
//!   │       ├── BulkCipher::seal_in_place() → tag
//!   │       ├── append tag to slab
//!   │       ├── drop(owned_plain) — zeroizes plaintext copy
//!   │       └── crossbeam::Sender::send(slab)
//!   │   })
//!   └── [returns immediately; encryption is async]
//!
//! Write loop (Tokio task, owns NoiseWriter + write half):
//!   ├── tokio::sync::mpsc::Receiver::recv() → frame batch
//!   ├── per frame: write [lane byte] + write_frame(body)
//!   └── flush
//! ```
//!
//! # Data flow (receive side)
//!
//! ```text
//! Socket read → lane byte check
//!   ├── 0x00 → Noise TransportState decrypt → dispatch
//!   └── 0x01..0x03 → BulkDispatcher
//!       ├── parse BulkFrameHeader
//!       ├── ReplayFilter::check_and_accept(nonce)
//!       └── rayon::spawn(move || {
//!           ├── PooledBuf(ct_and_tag) — zeroized + returned to recv pool on drop
//!           ├── BulkCipher::open_in_place(ct_and_tag)
//!           ├── plaintext → PooledBuf in DecryptedChunk
//!           └── crossbeam::Sender::send(DecryptedChunk)
//!       })
//!       → Reassembler (BTreeMap reorder)
//!           ├── MerkleDigest::feed_chunk_digest(pre-computed)
//!           ├── plaintext moved (PooledBuf) into ReassembledChunk
//!           └── deliver in-order to application (zeroized + returned to pool on drop)
//! ```

pub mod cipher;
pub mod dispatcher;
#[allow(unsafe_code)]
pub mod encrypt;
pub mod frame;
pub mod kdf;
pub mod pool;
pub mod pooled_buf;
pub mod reader;
pub mod reassembly;
pub mod replay;
pub mod session;
pub mod capability;
pub mod nonce;
pub mod stream;
pub mod transfer;
pub mod verify;
pub mod writer;

#[cfg(feature = "bulk-uring")]
#[allow(unsafe_code)]
pub mod uring_writer;

#[cfg(feature = "bulk-uring")]
#[allow(unsafe_code)]
pub mod uring_recv;

pub mod lending;

#[allow(unsafe_code)]
pub mod hugepage_pool;

/// memfd zero-copy path for 100Gbps same-host transfers.
/// Uses libc FFI (memfd_create, mmap, munmap, ftruncate, fcntl)
/// and unsafe Send/Sync impl on MemfdMapping. Each unsafe block
/// has a SAFETY annotation. Gated behind `bulk-memfd` feature.
#[cfg(feature = "bulk-memfd")]
#[allow(unsafe_code)]
pub mod memfd;

pub use cipher::BulkCipher;
pub use dispatcher::{BulkDispatcher, DecryptedChunk};
pub use encrypt::build_encrypt_pool;
pub use frame::{BulkFrameHeader, FrameKind, HEADER_LEN, MAX_CHUNK_PLAIN, TAG_LEN};
pub use pool::BufferPool;
pub use pooled_buf::PooledBuf;
pub use reader::{read_lane_frame, write_lane_byte};
pub use reassembly::{Reassembler, ReassembledChunk};
pub use replay::ReplayFilter;
pub use nonce::NonceCounter;
pub use session::BulkSession;
pub use stream::BulkStream;
pub use stream::BulkStreamId;
pub use lending::ZeroizingStream;
pub use transfer::{send_payload, BulkTransferAccumulator};
pub use verify::{StreamingDigest, MerkleDigest, DigestAlgorithm, merkle_root, merkle_root_with_algorithm, digest_oneshot, blake3_oneshot};

/// Shared atomic counters for bulk transfer observability.
///
/// One `Arc<BulkCounters>` is constructed at daemon startup and cloned
/// into both `ServerState` (write side — connection handlers increment)
/// and `DaemonContext` (read side — status endpoint, Prometheus, TUI).
/// Single source of truth. No duplication. No drift.
pub struct BulkCounters {
    pub frames_sent: std::sync::atomic::AtomicU64,
    pub frames_received: std::sync::atomic::AtomicU64,
    pub bytes_sent: std::sync::atomic::AtomicU64,
    pub bytes_received: std::sync::atomic::AtomicU64,
}

static_assertions::assert_impl_all!(BulkCounters: Send, Sync);

impl BulkCounters {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            frames_sent: std::sync::atomic::AtomicU64::new(0),
            frames_received: std::sync::atomic::AtomicU64::new(0),
            bytes_sent: std::sync::atomic::AtomicU64::new(0),
            bytes_received: std::sync::atomic::AtomicU64::new(0),
        })
    }
}

use std::sync::Arc;
