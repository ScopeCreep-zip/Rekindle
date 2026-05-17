//! Transport-internal control frames, outcome types, and connection state.
//!
//! Tag byte partition on lane 0x00 (Noise-encrypted payloads):
//! - 0x01..0x7F: transport-internal (intercepted, never reach FrameRouter)
//! - 0x80: application frame prefix
//!
//! Application frame wire format after Noise decryption:
//!   [tag=0x80][seq: u64 LE][application payload bytes]
//! The seq is assigned by the sender and echoed in the Ack.
//!
//! Bulk lane byte: read from frame_body[1] which is the cleartext
//! FrameKind field of the BulkFrameHeader. The header is authenticated
//! (AAD) but NOT encrypted, so frame_body[1] is always the kind byte.
//! BulkData=0x01, BulkFin=0x02, WindowUpdate=0x03 match lane values by design.

use std::time::Duration;
use serde::{Deserialize, Serialize};
use crate::error::IpcError;

// ---- Tag byte constants ----

pub struct TransportTag;

impl TransportTag {
    pub const ACK: u8 = 0x01;
    pub const HEARTBEAT_PING: u8 = 0x02;
    pub const HEARTBEAT_PONG: u8 = 0x03;
    pub const BULK_ACK: u8 = 0x04;
    pub const BULK_NACK: u8 = 0x05;
    pub const BULK_CANCEL: u8 = 0x06;
    pub const SHUTDOWN: u8 = 0x07;
    pub const SHUTDOWN_ACK: u8 = 0x08;
    pub const APP: u8 = 0x80;

    #[inline]
    pub fn is_transport(tag: u8) -> bool {
        tag > 0 && tag < Self::APP
    }

    #[inline]
    pub fn is_application(tag: u8) -> bool {
        tag == Self::APP
    }
}

// ---- Connection lifecycle ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionPhase {
    Handshaking,
    Ready,
    Active,
    Degraded,
    Dead,
    Draining,
    Closing,
    Closed,
}

impl ConnectionPhase {
    pub fn can_send(self) -> bool {
        matches!(self, Self::Ready | Self::Active | Self::Degraded)
    }

    pub fn can_recv(self) -> bool {
        matches!(self, Self::Ready | Self::Active | Self::Degraded | Self::Draining)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Dead | Self::Closed)
    }

    pub fn can_transition_to(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Handshaking, Self::Ready)
                | (Self::Handshaking, Self::Closed)
                | (Self::Ready, Self::Active)
                | (Self::Ready, Self::Closed)
                | (Self::Active, Self::Degraded)
                | (Self::Active, Self::Draining)
                | (Self::Active, Self::Dead)
                | (Self::Active, Self::Closed)
                | (Self::Degraded, Self::Active)
                | (Self::Degraded, Self::Dead)
                | (Self::Degraded, Self::Draining)
                | (Self::Degraded, Self::Closed)
                | (Self::Dead, Self::Closed)
                | (Self::Draining, Self::Closing)
                | (Self::Draining, Self::Dead)
                | (Self::Draining, Self::Closed)
                | (Self::Closing, Self::Closed)
        )
    }
}

impl std::fmt::Display for ConnectionPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Handshaking => "handshaking",
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Degraded => "degraded",
            Self::Dead => "dead",
            Self::Draining => "draining",
            Self::Closing => "closing",
            Self::Closed => "closed",
        })
    }
}

// ---- Send outcome ----

#[derive(Debug)]
pub enum SendOutcome {
    Delivered,
    AckTimeout,
    WriteFailed(IpcError),
    ConnectionNotActive,
}

impl SendOutcome {
    pub fn is_delivered(&self) -> bool {
        matches!(self, Self::Delivered)
    }
}

// ---- Bulk outcome ----

#[derive(Debug)]
pub enum BulkOutcome {
    Delivered {
        bytes_transferred: u64,
        duration: Duration,
        chunks: u64,
    },
    AckTimeout {
        bytes_sent: u64,
        chunks_sent: u64,
    },
    WriteFailed {
        bytes_sent: u64,
        chunks_sent: u64,
        error: IpcError,
    },
    IntegrityFailed,
    ConnectionLost,
    Cancelled,
    /// The stream_id is already in use by an in-flight bulk transfer.
    /// Wait for the current transfer to complete (Delivered/AckTimeout/etc.)
    /// before starting another on the same stream_id, or use a different
    /// stream_id for concurrent transfers.
    StreamBusy,
}

impl BulkOutcome {
    pub fn is_delivered(&self) -> bool {
        matches!(self, Self::Delivered { .. })
    }

    pub fn is_stream_busy(&self) -> bool {
        matches!(self, Self::StreamBusy)
    }
}

// ---- Bulk nack reasons ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BulkNackReason {
    DigestMismatch,
    DecryptFailed { chunk_seq: u32 },
    ReassemblyOverflow,
    Cancelled,
}

// ---- Read task frame types ----

/// A complete frame read by the dedicated read task and sent to the
/// control loop via channel. No async I/O in the control loop.
pub enum ReadFrame {
    /// Noise-decrypted control-lane payload (includes tag byte at [0]).
    Control(bytes::Bytes),
    /// Bulk-lane frame body (cleartext header + encrypted payload + tag).
    Bulk(Vec<u8>),
    /// Read error or clean EOF.
    Disconnected,
}

// ---- Transport frame serialization ----

/// Tag an application frame with seq for ack tracking.
/// Wire format: [0x80][seq: u64 LE][payload]
pub fn tag_application_frame(seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 8 + payload.len());
    buf.push(TransportTag::APP);
    buf.extend_from_slice(&seq.to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Extract seq from a received application frame.
/// Input is the full Noise-decrypted payload starting with tag=0x80.
/// Returns (seq, application_payload_slice).
pub fn parse_application_frame(data: &[u8]) -> Option<(u64, &[u8])> {
    if data.len() < 9 || data[0] != TransportTag::APP {
        return None;
    }
    let seq = u64::from_le_bytes(data[1..9].try_into().ok()?);
    Some((seq, &data[9..]))
}

pub fn encode_ack(seq: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TransportTag::ACK);
    buf.extend_from_slice(&seq.to_le_bytes());
    buf
}

pub fn encode_heartbeat_ping(timestamp_ms: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TransportTag::HEARTBEAT_PING);
    buf.extend_from_slice(&timestamp_ms.to_le_bytes());
    buf
}

pub fn encode_heartbeat_pong(echo_timestamp_ms: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TransportTag::HEARTBEAT_PONG);
    buf.extend_from_slice(&echo_timestamp_ms.to_le_bytes());
    buf
}

pub fn encode_bulk_ack(stream_id: u8, chunks: u64, bytes: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(18);
    buf.push(TransportTag::BULK_ACK);
    buf.push(stream_id);
    buf.extend_from_slice(&chunks.to_le_bytes());
    buf.extend_from_slice(&bytes.to_le_bytes());
    buf
}

pub fn encode_bulk_nack(stream_id: u8, reason: &BulkNackReason) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32);
    buf.push(TransportTag::BULK_NACK);
    buf.push(stream_id);
    buf.extend_from_slice(&postcard::to_allocvec(reason).unwrap_or_default());
    buf
}

pub fn encode_bulk_cancel(stream_id: u8, next_nonce: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(10);
    buf.push(TransportTag::BULK_CANCEL);
    buf.push(stream_id);
    buf.extend_from_slice(&next_nonce.to_le_bytes());
    buf
}

pub fn encode_shutdown() -> Vec<u8> {
    vec![TransportTag::SHUTDOWN]
}

pub fn encode_shutdown_ack() -> Vec<u8> {
    vec![TransportTag::SHUTDOWN_ACK]
}

/// Extract the lane byte for a bulk frame from the cleartext header.
/// frame_body[1] is the FrameKind byte (BulkData=0x01, BulkFin=0x02, WindowUpdate=0x03).
/// The header is authenticated (AAD) but not encrypted.
pub fn bulk_lane_byte(frame_body: &[u8]) -> u8 {
    if frame_body.len() > 1 { frame_body[1] } else { 0x01 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_transitions() {
        assert!(ConnectionPhase::Handshaking.can_transition_to(ConnectionPhase::Ready));
        assert!(ConnectionPhase::Active.can_transition_to(ConnectionPhase::Degraded));
        assert!(ConnectionPhase::Degraded.can_transition_to(ConnectionPhase::Active));
        assert!(!ConnectionPhase::Closed.can_transition_to(ConnectionPhase::Active));
        assert!(!ConnectionPhase::Handshaking.can_transition_to(ConnectionPhase::Active));
    }

    #[test]
    fn tag_partition() {
        assert!(TransportTag::is_transport(0x01));
        assert!(TransportTag::is_transport(0x7F));
        assert!(!TransportTag::is_transport(0x80));
        assert!(TransportTag::is_application(0x80));
        assert!(!TransportTag::is_application(0x01));
    }

    #[test]
    fn app_frame_roundtrip() {
        let tagged = tag_application_frame(42, b"hello");
        let (seq, payload) = parse_application_frame(&tagged).unwrap();
        assert_eq!(seq, 42);
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn ack_encoding() {
        let encoded = encode_ack(42);
        assert_eq!(encoded[0], TransportTag::ACK);
        assert_eq!(u64::from_le_bytes(encoded[1..9].try_into().unwrap()), 42);
    }
}
