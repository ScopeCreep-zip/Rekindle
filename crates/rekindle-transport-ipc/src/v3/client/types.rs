//! Client-side types — errors, success structs, inbound frame types.
//!
//! All operations return `Result<SuccessType, ErrorType>`. No flat enums
//! mixing success and failure. Callers use `?` for the common path and
//! match on the error for specific failure handling.

use std::time::Duration;

use tokio::sync::oneshot;

use crate::v3::io::lane_channels::PlaintextBuf;
use crate::v3::wire::clearance::Clearance;
use crate::v3::wire::lane::Lane;

// ── Client lifecycle errors ──────────────────────────────────────

/// Errors from client connect/handshake operations.
#[derive(Debug)]
pub enum ClientError {
    Connect(std::io::Error),
    Handshake(String),
    Send(String),
    UcredFailed(String),
    PrologueFailed(String),
    CipherInit(String),
    ConnectionClosed,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "connect failed: {e}"),
            Self::Handshake(s) => write!(f, "handshake failed: {s}"),
            Self::Send(s) => write!(f, "send failed: {s}"),
            Self::UcredFailed(s) => write!(f, "UCred extraction failed: {s}"),
            Self::PrologueFailed(s) => write!(f, "prologue construction failed: {s}"),
            Self::CipherInit(s) => write!(f, "cipher initialization failed: {s}"),
            Self::ConnectionClosed => write!(f, "connection closed"),
        }
    }
}

impl std::error::Error for ClientError {}

// ── Send operation types ─────────────────────────────────────────

/// Successful delivery of a control-plane frame.
#[derive(Debug)]
pub struct SendDelivered;

/// Errors from a control-plane send operation.
#[derive(Debug)]
pub enum SendError {
    AckTimeout,
    WriteFailed(String),
    ConnectionNotActive,
    Rejected { reason_code: u32, detail: String },
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AckTimeout => write!(f, "ack timeout"),
            Self::WriteFailed(s) => write!(f, "write failed: {s}"),
            Self::ConnectionNotActive => write!(f, "connection not active"),
            Self::Rejected { reason_code, detail } =>
                write!(f, "peer rejected request (code {reason_code}): {detail}"),
        }
    }
}

impl std::error::Error for SendError {}

// ── Request-reply types ─────────────────────────────────────────

/// The decoded reply payload from a `request_reply()` call.
///
/// Contains the application-level payload bytes (already decoded from
/// wire format by the transport) and the transport-level status_phase.
/// The caller deserializes `payload` with their own codec (e.g., postcard
/// for `DaemonResponse`).
#[derive(Debug)]
pub struct ReplyPayload {
    /// The application payload from the reply.
    pub payload: Vec<u8>,
    /// Transport-level status phase from the reply frame.
    pub status_phase: u32,
}

/// Errors from a `request_reply()` operation.
#[derive(Debug)]
pub enum RequestReplyError {
    /// The reply was not received within the timeout.
    Timeout,
    /// The connection is not in an active phase.
    ConnectionNotActive,
    /// The request was rejected by the peer.
    Rejected { reason_code: u32, detail: String },
    /// The reply channel was dropped (connection lost).
    ChannelClosed,
    /// The outbound channel failed (control loop dead).
    WriteFailed(String),
}

impl std::fmt::Display for RequestReplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "request-reply timeout"),
            Self::ConnectionNotActive => write!(f, "connection not active"),
            Self::Rejected { reason_code, detail } =>
                write!(f, "peer rejected request (code {reason_code}): {detail}"),
            Self::ChannelClosed => write!(f, "reply channel closed"),
            Self::WriteFailed(s) => write!(f, "write failed: {s}"),
        }
    }
}

impl std::error::Error for RequestReplyError {}

/// Oneshot sender type for per-request reply delivery.
///
/// Used by `ClientRouter::on_reply` and `SendState::request_reply`.
/// Defined once here to avoid repeating the full generic signature
/// at every construction and storage site.
pub type ReplySender = oneshot::Sender<Result<ReplyPayload, RequestReplyError>>;

// ── Bulk transfer types ──────────────────────────────────────────

/// Successful delivery of a bulk data transfer.
#[derive(Debug)]
pub struct BulkDelivered {
    pub bytes_transferred: u64,
    pub duration: Duration,
    pub chunks: u64,
}

/// Errors from a bulk data transfer.
#[derive(Debug)]
pub enum BulkError {
    AckTimeout { bytes_sent: u64, chunks_sent: u64 },
    WriteFailed { bytes_sent: u64, chunks_sent: u64, detail: String },
    IntegrityFailed,
    ConnectionLost,
    PeerRejected { reason: String },
    Cancelled,
    StreamBusy,
}

impl std::fmt::Display for BulkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AckTimeout { bytes_sent, chunks_sent } =>
                write!(f, "bulk ack timeout: {bytes_sent} bytes, {chunks_sent} chunks sent"),
            Self::WriteFailed { detail, .. } => write!(f, "bulk write failed: {detail}"),
            Self::IntegrityFailed => write!(f, "content hash verification failed"),
            Self::ConnectionLost => write!(f, "connection lost during transfer"),
            Self::PeerRejected { reason } => write!(f, "peer rejected transfer: {reason}"),
            Self::Cancelled => write!(f, "transfer cancelled"),
            Self::StreamBusy => write!(f, "stream_id already in use"),
        }
    }
}

impl std::error::Error for BulkError {}

// ── Inbound frame types ──────────────────────────────────────────

/// A decoded inbound frame delivered to the client application.
#[derive(Debug)]
pub struct InboundFrame {
    pub lane: Lane,
    pub class: u8,
    pub kind: u8,
    pub payload: Vec<u8>,
    pub conn_id: u64,
    pub session_id: uuid::Uuid,
    pub peer_id: [u8; 32],
    pub message_id: Option<uuid::Uuid>,
    pub correlation_id: Option<uuid::Uuid>,
    pub sender_clearance: Option<Clearance>,
    pub subscription_id: Option<uuid::Uuid>,
    pub topic_hash: Option<[u8; 32]>,
    pub event_seq: Option<u32>,
    pub status_phase: Option<u32>,
    pub transfer_id: Option<uuid::Uuid>,
    pub stream_id: Option<u8>,
    pub chunk_index: Option<u32>,
}

// ── Bulk chunk — zero-copy streaming delivery ────────────────────

/// A single bulk data chunk received from the peer (streaming API).
///
/// The application calls `chunk.payload()` to get `&[u8]`.
/// When `is_last` is true, the transfer is complete (content hash
/// verified) and `payload()` returns `&[]`.
pub struct BulkChunk {
    pub stream_id: u8,
    pub chunk_seq: u32,
    pub is_last: bool,
    data: ChunkData,
}

enum ChunkData {
    Payload(PlaintextBuf),
    Empty,
}

impl BulkChunk {
    /// The decrypted plaintext payload. Returns `&[]` for the completion
    /// sentinel (`is_last == true`).
    pub fn payload(&self) -> &[u8] {
        match &self.data {
            ChunkData::Payload(buf) => buf,
            ChunkData::Empty => &[],
        }
    }

    /// Consume the chunk and take ownership of the plaintext buffer.
    /// Returns None for the completion sentinel. When the returned
    /// PlaintextBuf drops, pooled buffers return to the recv freelist.
    pub fn into_plaintext(self) -> Option<PlaintextBuf> {
        match self.data {
            ChunkData::Payload(buf) => Some(buf),
            ChunkData::Empty => None,
        }
    }

    /// Construct a data chunk from a PlaintextBuf (transport-internal).
    pub(crate) fn from_plaintext(stream_id: u8, chunk_seq: u32, buf: PlaintextBuf) -> Self {
        Self { stream_id, chunk_seq, is_last: false, data: ChunkData::Payload(buf) }
    }

    /// Construct a completion sentinel (transport-internal).
    pub(crate) fn completion(stream_id: u8, chunk_seq: u32) -> Self {
        Self { stream_id, chunk_seq, is_last: true, data: ChunkData::Empty }
    }
}

impl std::fmt::Debug for BulkChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BulkChunk")
            .field("stream_id", &self.stream_id)
            .field("chunk_seq", &self.chunk_seq)
            .field("payload_len", &self.payload().len())
            .field("is_last", &self.is_last)
            .finish()
    }
}

// ── Connection lifecycle ─────────────────────────────────────────

/// Connection lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientPhase {
    Ready,
    Active,
    Degraded,
    Dead,
    Closed,
}

impl ClientPhase {
    pub fn can_send(self) -> bool {
        matches!(self, Self::Ready | Self::Active | Self::Degraded)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Dead | Self::Closed)
    }
}
