//! Exhaustive error taxonomy for all transport operations.
//!
//! Every variant is specific, actionable, and carries enough context to
//! diagnose without a debugger. No `String`-only catch-all variants.

use thiserror::Error;

/// Transport-layer error covering every failure mode across node lifecycle,
/// routing, send/receive, DHT, crypto, and gossip operations.
#[derive(Debug, Error)]
pub enum TransportError {
    // ── Node lifecycle ───────────────────────────────────────────────

    /// The transport node has not been started yet.
    #[error("transport node not started")]
    NotStarted,

    /// Attempted to start a node that is already running.
    #[error("transport node already started")]
    AlreadyStarted,

    /// Failed to attach to the Veilid network.
    #[error("attach failed: {reason}")]
    AttachFailed { reason: String },

    /// Failed to shut down the transport node gracefully.
    #[error("shutdown failed: {reason}")]
    ShutdownFailed { reason: String },

    /// The network is not yet ready for operations (public_internet_ready = false).
    #[error("network not ready")]
    NetworkNotReady,

    // ── Routing ──────────────────────────────────────────────────────

    /// No route available for the target peer.
    #[error("no route for peer {peer}")]
    NoRoute { peer: String },

    /// Failed to import a remote peer's private route blob.
    #[error("route import failed for {peer}: {reason}")]
    RouteImportFailed { peer: String, reason: String },

    /// The route for this peer has expired or been reported dead.
    #[error("route expired for peer {peer}")]
    RouteExpired { peer: String },

    /// Circuit breaker is open — too many consecutive failures for this peer.
    #[error("circuit open for peer {peer} (failures: {failures}, cooldown remaining)")]
    CircuitOpen { peer: String, failures: u32 },

    /// Failed to allocate a private route for receiving messages.
    #[error("route allocation failed: {reason}")]
    RouteAllocationFailed { reason: String },

    // ── Send ─────────────────────────────────────────────────────────

    /// The serialized payload exceeds the Veilid maximum (32,764 bytes with frame header).
    #[error("payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: usize, max: usize },

    /// The underlying Veilid send operation failed.
    #[error("send to {target} failed: {reason}")]
    SendFailed { target: String, reason: String },

    /// An RPC call (app_call) timed out waiting for a response.
    #[error("{operation} timed out after {duration_ms}ms")]
    Timeout { operation: String, duration_ms: u64 },

    // ── Receive ──────────────────────────────────────────────────────

    /// The inbound frame header is invalid (too short, corrupt).
    #[error("invalid frame: {reason}")]
    InvalidFrame { reason: String },

    /// The frame version byte is not recognized by this build.
    #[error("unknown protocol version {version}")]
    UnknownVersion { version: u8 },

    /// The frame type ID is not recognized by this build.
    #[error("unknown frame type 0x{type_id:02x}")]
    UnknownType { type_id: u8 },

    /// Ed25519 signature verification failed on an inbound message.
    #[error("signature verification failed for sender {sender}")]
    SignatureVerificationFailed { sender: String },

    /// Decryption of an inbound payload failed (wrong key, corrupt data).
    #[error("decryption failed: {reason}")]
    DecryptionFailed { reason: String },

    /// Deserialization of a typed payload failed after successful decryption.
    #[error("deserialization failed for type 0x{type_id:02x}: {reason}")]
    DeserializationFailed { type_id: u8, reason: String },

    /// Duplicate message detected by the dedup cache.
    #[error("duplicate message (dedup key: {dedup_key})")]
    DuplicateMessage { dedup_key: String },

    // ── DHT ──────────────────────────────────────────────────────────

    /// Attempted an operation on a DHT record that is not open.
    #[error("DHT record not open: {key}")]
    RecordNotOpen { key: String },

    /// Failed to create a new DHT record.
    #[error("DHT record creation failed: {reason}")]
    RecordCreateFailed { reason: String },

    /// The data being written to a subkey exceeds ValueData::MAX_LEN (32,768).
    #[error("subkey {subkey} data too large: {size} bytes (max {max})")]
    SubkeyTooLarge { subkey: u32, size: usize, max: usize },

    /// A write was rejected because the network has a newer sequence number.
    #[error("stale write on subkey {subkey}: local seq {local_seq}, network seq {network_seq}")]
    StaleWrite { subkey: u32, local_seq: u32, network_seq: u32 },

    /// A DHT watch died (count reached zero or empty subkeys reported).
    #[error("DHT watch died for record {key}")]
    WatchDied { key: String },

    /// A generic DHT operation failed.
    #[error("DHT error: {reason}")]
    DhtError { reason: String },

    // ── Crypto ────────────────────────────────────────────────────────

    /// No MEK available for encrypting/decrypting a channel message.
    #[error("no MEK for {community_id}/{channel_id}")]
    NoMekForChannel { community_id: String, channel_id: String },

    /// The MEK generation on a received message does not match any cached generation.
    #[error("MEK generation mismatch: expected {expected}, got {got}")]
    MekGenerationMismatch { expected: u64, got: u64 },

    /// No Signal Protocol session exists for the target peer.
    #[error("no Signal session for peer {peer}")]
    SignalSessionNotFound { peer: String },

    /// MEK unwrap (ECDH + AES-GCM decryption) failed.
    #[error("MEK unwrap failed: {reason}")]
    MekUnwrapFailed { reason: String },

    /// Encryption failed.
    #[error("encryption failed: {reason}")]
    EncryptionFailed { reason: String },

    // ── Serialization ────────────────────────────────────────────────

    /// Serialization of an outbound payload failed.
    #[error("serialization failed: {reason}")]
    SerializationFailed { reason: String },

    // ── Internal ─────────────────────────────────────────────────────

    /// An internal invariant was violated. Should never happen in production.
    #[error("internal error: {0}")]
    Internal(String),
}

/// Convenience alias used throughout this crate.
pub type Result<T> = std::result::Result<T, TransportError>;
