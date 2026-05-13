//! Error types for the IPC subsystem.
//!
//! Every variant is specific and actionable. No catch-all `String` variants.
//! Error types preserve the OS error code where applicable — `ENOENT` and
//! `EACCES` are never conflated. [RC-1]

use thiserror::Error;

/// IPC subsystem errors.
#[derive(Debug, Error)]
pub enum IpcError {
    /// Socket bind failed (directory missing, permissions, or stale socket).
    #[error("socket bind failed at {path}: {source}")]
    SocketBind {
        path: String,
        source: std::io::Error,
    },

    /// Socket directory creation failed.
    #[error("failed to create socket directory {path}: {source}")]
    DirectoryCreate {
        path: String,
        source: std::io::Error,
    },

    /// UCred extraction failed — connection must be rejected.
    #[error("UCred extraction failed: {0}")]
    UcredFailed(std::io::Error),

    /// UID mismatch — cross-user connection attempt.
    #[error("UID mismatch: peer UID {peer_uid}, expected {expected_uid}")]
    UidMismatch { peer_uid: u32, expected_uid: u32 },

    /// Noise handshake failed (timeout, protocol error, or key mismatch).
    #[error("Noise handshake failed: {reason}")]
    HandshakeFailed { reason: String },

    /// Noise handshake timed out.
    #[error("Noise handshake timed out after {timeout_ms}ms")]
    HandshakeTimeout { timeout_ms: u64 },

    /// IPC request timed out waiting for a response from the daemon.
    #[error("IPC request timed out after {timeout_ms}ms")]
    RequestTimeout { timeout_ms: u64 },

    /// Postcard serialization failed.
    #[error("serialization failed: {reason}")]
    SerializationFailed { reason: String },

    /// Postcard deserialization failed on untrusted input.
    #[error("deserialization failed: {reason}")]
    DeserializationFailed { reason: String },

    /// Frame exceeds maximum allowed size.
    #[error("frame size {size} exceeds maximum {max}")]
    FrameTooLarge { size: u32, max: u32 },

    /// Noise encrypt failed.
    #[error("Noise encrypt failed: {reason}")]
    EncryptFailed { reason: String },

    /// Noise decrypt failed.
    #[error("Noise decrypt failed: {reason}")]
    DecryptFailed { reason: String },

    /// Chunk count header invalid.
    #[error("invalid chunk count header: expected 4 bytes, got {got}")]
    InvalidChunkHeader { got: usize },

    /// Too many chunks in encrypted frame.
    #[error("chunk count {count} exceeds maximum {max}")]
    TooManyChunks { count: usize, max: usize },

    /// I/O error on the socket.
    #[error("socket I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Connection was closed by the peer.
    #[error("connection closed")]
    ConnectionClosed,

    /// Outbound channel is closed (client shut down).
    #[error("outbound channel closed")]
    OutboundClosed,

    /// Rate limit exceeded for this agent.
    #[error("rate limit exceeded for agent {agent_name}")]
    RateLimitExceeded { agent_name: String },

    /// Agent identity changed mid-session (security violation).
    #[error("agent identity changed mid-session: expected {expected}, got {got}")]
    IdentityMismatch { expected: String, got: String },

    /// Sender clearance below message security level.
    #[error("sender clearance {sender_level:?} below message level {msg_level:?}")]
    ClearanceInsufficient {
        sender_level: super::message::SecurityLevel,
        msg_level: super::message::SecurityLevel,
    },

    /// Agent name contains invalid characters (path traversal prevention).
    #[error("invalid agent name '{name}': must match [a-zA-Z0-9_-]+")]
    InvalidAgentName { name: String },

    /// Key file tamper detection failed.
    #[error("TAMPER DETECTED: {agent} keypair checksum mismatch — delete {path} and restart")]
    KeyTamperDetected { agent: String, path: String },

    /// Frame too short to contain required header fields.
    #[error("frame too short: {len} bytes (minimum {min} required)")]
    FrameTooShort { len: usize, min: usize },

    /// Unknown lane byte received on the wire.
    #[error("unknown lane byte 0x{lane:02x} — peer may be running an incompatible protocol version")]
    UnknownLaneByte { lane: u8 },
}

/// Convenience type alias.
pub type Result<T> = std::result::Result<T, IpcError>;
