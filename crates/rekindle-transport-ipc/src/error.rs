//! Typed error taxonomy for the IPC transport.
//!
//! Every variant is specific and actionable. No catch-all `String` variants.
//! `is_retriable()` classifies errors for retry logic per the AF_UNIX kernel
//! semantics: EAGAIN on backlog-full (not ECONNREFUSED), EMFILE recoverable
//! via reserve-fd pattern.

use thiserror::Error;

/// IPC transport errors.
#[derive(Debug, Error)]
pub enum IpcError {
    /// Socket bind failed (directory missing, permissions, stale socket).
    #[error("socket bind at {path}: {source}")]
    SocketBind { path: String, source: std::io::Error },

    /// Socket directory creation failed.
    #[error("directory create {path}: {source}")]
    DirectoryCreate { path: String, source: std::io::Error },

    /// UCred extraction failed — connection must be rejected.
    #[error("UCred extraction: {0}")]
    UcredFailed(std::io::Error),

    /// UID mismatch — cross-user connection attempt.
    #[error("UID mismatch: peer {peer_uid}, expected {expected_uid}")]
    UidMismatch { peer_uid: u32, expected_uid: u32 },

    /// Noise handshake failed (timeout, protocol error, key mismatch).
    #[error("Noise handshake: {reason}")]
    HandshakeFailed { reason: String },

    /// Noise handshake timed out.
    #[error("handshake timed out after {timeout_ms}ms")]
    HandshakeTimeout { timeout_ms: u64 },

    /// IPC request timed out waiting for response.
    #[error("request timed out after {timeout_ms}ms")]
    RequestTimeout { timeout_ms: u64 },

    /// Postcard serialization failed.
    #[error("serialization: {reason}")]
    SerializationFailed { reason: String },

    /// Postcard deserialization failed on untrusted input.
    #[error("deserialization: {reason}")]
    DeserializationFailed { reason: String },

    /// Frame exceeds maximum allowed size. Checked BEFORE allocation.
    #[error("frame {size} exceeds max {max}")]
    FrameTooLarge { size: u32, max: u32 },

    /// Frame too short to contain required fields.
    #[error("frame too short: {len} bytes (min {min})")]
    FrameTooShort { len: usize, min: usize },

    /// Noise encrypt failed.
    #[error("encrypt: {reason}")]
    EncryptFailed { reason: String },

    /// Noise decrypt failed.
    #[error("decrypt: {reason}")]
    DecryptFailed { reason: String },

    /// Chunk count header invalid.
    #[error("invalid chunk header: expected 4 bytes, got {got}")]
    InvalidChunkHeader { got: usize },

    /// Too many chunks in encrypted frame.
    #[error("chunk count {count} exceeds max {max}")]
    TooManyChunks { count: usize, max: usize },

    /// I/O error on the socket.
    #[error("socket I/O: {0}")]
    Io(#[from] std::io::Error),

    /// Connection closed by the peer.
    #[error("connection closed")]
    ConnectionClosed,

    /// Outbound channel closed (client shut down).
    #[error("outbound channel closed")]
    OutboundClosed,

    /// Rate limit exceeded for this agent.
    #[error("rate limit exceeded for {agent_name}")]
    RateLimitExceeded { agent_name: String },

    /// Unknown lane byte on the wire.
    #[error("unknown lane 0x{lane:02x}")]
    UnknownLaneByte { lane: u8 },

    /// Backpressure: too many bytes in flight.
    #[error("backpressure: {buffered} bytes in flight")]
    Backpressure { buffered: u64 },

    /// Agent name contains invalid characters (path traversal prevention).
    #[error("invalid agent name '{name}': must match [a-zA-Z0-9_-]+")]
    InvalidAgentName { name: String },

    /// Key file tamper detection failed.
    #[error("TAMPER DETECTED: {agent} keypair checksum mismatch at {path}")]
    KeyTamperDetected { agent: String, path: String },
}

/// Convenience type alias.
pub type IpcResult<T> = Result<T, IpcError>;

impl IpcError {
    /// Classifies whether this error is retriable.
    ///
    /// AF_UNIX kernel semantics: EAGAIN on backlog-full (Agent 1),
    /// EMFILE recoverable via reserve-fd pattern, ECONNRESET is
    /// retriable (reconnect). EPIPE/BrokenPipe and PermissionDenied
    /// are fatal.
    pub fn is_retriable(&self) -> bool {
        match self {
            Self::Io(e) => {
                matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::ConnectionReset
                ) || matches!(
                    e.raw_os_error(),
                    Some(libc::EMFILE) | Some(libc::ENOMEM) | Some(libc::EAGAIN)
                )
            }
            Self::Backpressure { .. } | Self::RateLimitExceeded { .. } => true,
            Self::RequestTimeout { .. } | Self::HandshakeTimeout { .. } => true,
            _ => false,
        }
    }
}
