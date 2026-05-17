#![deny(unsafe_code)]
//! Encrypted IPC bus transport for the Rekindle platform.
//!
//! Zero knowledge of application protocols. Carries any
//! `T: Serialize + DeserializeOwned` payload over Noise-IK-encrypted
//! Unix domain sockets with a parallel AES-256-GCM bulk data plane.
//!
//! # Transport guarantees
//!
//! - **Ack-based delivery:** `send_frame` returns `Future<SendOutcome>` that
//!   resolves when the peer acks, times out, or the write fails. No fire-and-forget.
//! - **Bulk transfer lifecycle:** `send_bulk` returns `Future<BulkOutcome>` with
//!   Delivered/AckTimeout/WriteFailed/IntegrityFailed/ConnectionLost/Cancelled.
//! - **Heartbeat:** transport-level ping/pong detects dead peers within 14s.
//! - **Ordered shutdown:** Shutdown/ShutdownAck frame exchange with drain timeout.
//! - **Error propagation:** every error reaches the caller. No `let _ =` on sends.
//!
//! # Wire format
//!
//! All frames on lane 0x00 carry a 1-byte tag after Noise decryption:
//! - `0x01..0x7F`: transport-internal (Ack, Heartbeat, BulkAck/Nack, Shutdown)
//! - `0x80..0xFF`: application frames (delivered to FrameRouter)
//!
//! Bulk data flows on lanes 0x01-0x03, bypassing Noise.

pub mod error;
pub mod config;
pub mod socket;
pub mod frame;
pub mod noise;
pub mod envelope;
pub mod bulk;
pub mod server;
pub mod client;
pub mod backpressure;
pub mod transport_frame;

// ---- Public API re-exports ----

pub use error::{IpcError, IpcResult};
pub use config::IpcConfig;
pub use socket::{PeerCredentials, socket_path, runtime_dir};
pub use envelope::{
    Message, MessageContext, SecurityLevel, AgentType, Timestamp,
    RoutingHeader, SharedFrame, WIRE_VERSION,
};
pub use noise::{NoiseTransport, NoiseReader, NoiseWriter};
pub use noise::keys::{ZeroizingKeypair, generate_keypair, NOISE_PARAMS};
pub use frame::codec::{encode_frame, decode_frame, MAX_FRAME_SIZE};
pub use bulk::{
    BulkCipher, BulkStream, ZeroizingStream,
    BulkCounters, NonceCounter, BufferPool, ZeroizingBuf, DigestAlgorithm,
    BulkKeyPair,
};
pub use bulk::transfer::{send_payload, BulkTransferAccumulator};
pub use server::{IpcServer, FrameRouter};
pub use client::{IpcClient, BulkChunk, SharedPools};
pub use backpressure::GlobalMemoryGuard;
pub use transport_frame::{
    TransportTag, SendOutcome, BulkOutcome, BulkNackReason, ConnectionPhase,
};
