//! Deterministic wire frame encoding and decoding.
//!
//! Every byte that enters or leaves the Veilid network is wrapped in a
//! 4-byte frame header:
//!
//! ```text
//! ┌─────────┬─────────┬──────────────┬───────────────────────────────┐
//! │ version │ type_id │ payload_len  │           payload             │
//! │  1 byte │ 1 byte  │  2 bytes BE  │     0..32764 bytes            │
//! └─────────┴─────────┴──────────────┴───────────────────────────────┘
//! ```
//!
//! - **version**: Protocol version. `0x01` for the initial release. Receivers
//!   that don't recognize the version MUST drop and log.
//! - **type_id**: Payload type from [`TypeId`]. Determines deserialization
//!   codec and crypto expectations.
//! - **payload_len**: Length of `payload` in bytes, big-endian u16.
//!   Maximum value: 32,764 (32,768 Veilid limit minus 4 byte header).
//! - **payload**: Serialized, optionally encrypted content.

use crate::error::{TransportError, Result};

/// Current protocol version.
pub const PROTOCOL_VERSION: u8 = 0x01;

/// Frame header size in bytes.
pub const HEADER_SIZE: usize = 4;

/// Maximum payload size (Veilid's 32,768 byte limit minus 4 byte header).
pub const MAX_PAYLOAD_SIZE: usize = 32_764;

/// Payload type identifiers.
///
/// Each variant maps to a specific serialization format, crypto expectations,
/// and handler method. The numeric values are stable across protocol versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TypeId {
    // ── DM (peer-to-peer, Signal Protocol encrypted) ─────────────
    DmMessage         = 0x01,
    DmTyping          = 0x02,
    FriendRequest     = 0x03,
    FriendAccept      = 0x04,
    FriendReject      = 0x05,
    FriendRequestAck  = 0x06,
    Unfriend          = 0x07,
    UnfriendAck       = 0x08,
    ProfileKeyRotated = 0x09,
    DmPresenceUpdate  = 0x0A,

    // ── Community gossip (Ed25519 signed, optionally MEK encrypted) ─
    GossipBroadcast   = 0x10,

    // ── Voice (MEK encrypted, HMAC authenticated) ────────────────
    VoicePacket       = 0x11,

    // ── RPC request/response (app_call, Ed25519 signed) ──────────
    BootstrapRequest  = 0x20,
    BootstrapResponse = 0x21,
    MekTransfer       = 0x22,
    SyncRequest       = 0x23,
    SyncResponse      = 0x24,
    DmCall            = 0x25,
}

impl TypeId {
    /// Parse a raw byte into a TypeId, returning None for unknown values.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::DmMessage),
            0x02 => Some(Self::DmTyping),
            0x03 => Some(Self::FriendRequest),
            0x04 => Some(Self::FriendAccept),
            0x05 => Some(Self::FriendReject),
            0x06 => Some(Self::FriendRequestAck),
            0x07 => Some(Self::Unfriend),
            0x08 => Some(Self::UnfriendAck),
            0x09 => Some(Self::ProfileKeyRotated),
            0x0A => Some(Self::DmPresenceUpdate),
            0x10 => Some(Self::GossipBroadcast),
            0x11 => Some(Self::VoicePacket),
            0x20 => Some(Self::BootstrapRequest),
            0x21 => Some(Self::BootstrapResponse),
            0x22 => Some(Self::MekTransfer),
            0x23 => Some(Self::SyncRequest),
            0x24 => Some(Self::SyncResponse),
            0x25 => Some(Self::DmCall),
            _    => None,
        }
    }

    /// Whether this type requires Ed25519 signature verification on receive.
    #[allow(clippy::unused_self, clippy::trivially_copy_pass_by_ref)]
    pub fn requires_signature(&self) -> bool {
        // Every type requires signature verification. No exceptions.
        // Friend requests are TOFU-signed (self-asserted key), but still verified.
        true
    }

    /// Whether this type carries encrypted content that needs decryption.
    pub fn requires_decryption(self) -> bool {
        matches!(self,
            Self::DmMessage
            | Self::DmTyping
            | Self::DmPresenceUpdate
            | Self::FriendReject
            | Self::Unfriend
            | Self::UnfriendAck
            | Self::ProfileKeyRotated
            | Self::FriendRequestAck
            | Self::VoicePacket
        )
    }

    /// Whether this type is a gossip broadcast that participates in dedup.
    pub fn is_gossip(self) -> bool {
        self == Self::GossipBroadcast
    }

    /// Whether this type is an RPC (app_call) payload.
    pub fn is_rpc(self) -> bool {
        matches!(self,
            Self::BootstrapRequest
            | Self::BootstrapResponse
            | Self::MekTransfer
            | Self::SyncRequest
            | Self::SyncResponse
            | Self::DmCall
        )
    }
}

/// Encode a typed payload into a framed wire message.
///
/// Returns `Err(PayloadTooLarge)` if the payload exceeds [`MAX_PAYLOAD_SIZE`].
pub fn encode(type_id: TypeId, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(TransportError::PayloadTooLarge {
            size: payload.len(),
            max: MAX_PAYLOAD_SIZE,
        });
    }

    #[allow(clippy::cast_possible_truncation)] // guarded by MAX_PAYLOAD_SIZE check above
    let len = payload.len() as u16;
    let mut frame = Vec::with_capacity(HEADER_SIZE + payload.len());
    frame.push(PROTOCOL_VERSION);
    frame.push(type_id as u8);
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Decode a framed wire message into its type and payload.
///
/// Validates the version byte, type ID, and declared length against
/// actual data length. Returns the type ID and a slice of the payload.
///
/// # Errors
///
/// - `InvalidFrame` if the data is too short for a header.
/// - `UnknownVersion` if the version byte is not recognized.
/// - `UnknownType` if the type ID byte is not recognized.
/// - `InvalidFrame` if the declared length doesn't match available data.
pub fn decode(data: &[u8]) -> Result<(TypeId, &[u8])> {
    if data.len() < HEADER_SIZE {
        return Err(TransportError::InvalidFrame {
            reason: format!("frame too short: {} bytes (minimum {})", data.len(), HEADER_SIZE),
        });
    }

    let version = data[0];
    if version != PROTOCOL_VERSION {
        return Err(TransportError::UnknownVersion { version });
    }

    let type_id = TypeId::from_byte(data[1]).ok_or(TransportError::UnknownType {
        type_id: data[1],
    })?;

    let declared_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let available = data.len() - HEADER_SIZE;

    if declared_len > available {
        return Err(TransportError::InvalidFrame {
            reason: format!(
                "declared payload length {declared_len} exceeds available data {available}"
            ),
        });
    }

    Ok((type_id, &data[HEADER_SIZE..HEADER_SIZE + declared_len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encode_decode() {
        let payload = b"hello, rekindle";
        let frame = encode(TypeId::DmMessage, payload).unwrap();
        let (tid, decoded) = decode(&frame).unwrap();
        assert_eq!(tid, TypeId::DmMessage);
        assert_eq!(decoded, payload);
    }

    #[test]
    fn rejects_unknown_version() {
        let mut frame = encode(TypeId::DmMessage, b"test").unwrap();
        frame[0] = 0xFF;
        assert!(matches!(decode(&frame), Err(TransportError::UnknownVersion { version: 0xFF })));
    }

    #[test]
    fn rejects_unknown_type() {
        let mut frame = encode(TypeId::DmMessage, b"test").unwrap();
        frame[1] = 0xFE;
        assert!(matches!(decode(&frame), Err(TransportError::UnknownType { type_id: 0xFE })));
    }

    #[test]
    fn rejects_too_short() {
        assert!(matches!(decode(&[0x01, 0x01]), Err(TransportError::InvalidFrame { .. })));
    }

    #[test]
    fn rejects_payload_too_large() {
        let big = vec![0u8; MAX_PAYLOAD_SIZE + 1];
        assert!(matches!(encode(TypeId::DmMessage, &big), Err(TransportError::PayloadTooLarge { .. })));
    }

    #[test]
    fn empty_payload_roundtrip() {
        let frame = encode(TypeId::FriendReject, &[]).unwrap();
        let (tid, payload) = decode(&frame).unwrap();
        assert_eq!(tid, TypeId::FriendReject);
        assert!(payload.is_empty());
    }

    #[test]
    fn max_payload_roundtrip() {
        let payload = vec![0xAB; MAX_PAYLOAD_SIZE];
        let frame = encode(TypeId::GossipBroadcast, &payload).unwrap();
        let (tid, decoded) = decode(&frame).unwrap();
        assert_eq!(tid, TypeId::GossipBroadcast);
        assert_eq!(decoded.len(), MAX_PAYLOAD_SIZE);
    }

    #[test]
    fn all_type_ids_have_stable_byte_values() {
        // Guard against accidental reordering of the enum repr values.
        assert_eq!(TypeId::DmMessage as u8, 0x01);
        assert_eq!(TypeId::GossipBroadcast as u8, 0x10);
        assert_eq!(TypeId::VoicePacket as u8, 0x11);
        assert_eq!(TypeId::BootstrapRequest as u8, 0x20);
        assert_eq!(TypeId::DmCall as u8, 0x25);
    }

    #[test]
    fn from_byte_roundtrip_all_known() {
        for byte in 0..=0xFF_u8 {
            if let Some(tid) = TypeId::from_byte(byte) {
                assert_eq!(tid as u8, byte);
            }
        }
    }
}
