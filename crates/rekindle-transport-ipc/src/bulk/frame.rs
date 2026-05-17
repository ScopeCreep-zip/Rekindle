//! Wire frame format for the bulk transfer plane.
//!
//! ```text
//! [1B stream_id][1B kind][8B nonce LE][4B chunk_seq LE][N bytes ciphertext][16B AEAD tag]
//! ```
//!
//! Two independent indices per frame:
//! - `nonce`: globally unique per cipher key (AEAD cryptographic invariant)
//! - `chunk_seq`: per-stream sequential ordering (0-indexed per transfer)
//!
//! The lane byte and 4-byte length prefix are handled by the lane/framing
//! layers. This module defines the 14-byte bulk header inside the body.

/// Bulk header length: stream_id(1) + kind(1) + nonce(8) + chunk_seq(4) = 14.
pub const HEADER_LEN: usize = 14;

/// AES-256-GCM tag length.
pub const TAG_LEN: usize = 16;

/// Maximum plaintext per bulk chunk: 65535 - 16 = 65519.
/// Matches Noise MAXMSGLEN - TAGLEN for consistency.
pub const MAX_CHUNK_PLAIN: usize = 65_519;

/// Maximum frame body: header + max ciphertext + tag.
pub const MAX_FRAME_BODY: usize = HEADER_LEN + MAX_CHUNK_PLAIN + TAG_LEN;

/// Frame kind discriminant.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    /// Noise-encrypted control-plane message.
    Control = 0x00,
    /// Bulk data chunk.
    BulkData = 0x01,
    /// Final chunk (carries blob digest).
    BulkFin = 0x02,
    /// Flow control: receiver grants send credit.
    WindowUpdate = 0x03,
}

impl FrameKind {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Control),
            0x01 => Some(Self::BulkData),
            0x02 => Some(Self::BulkFin),
            0x03 => Some(Self::WindowUpdate),
            _ => None,
        }
    }

    /// True for bulk-plane kinds (not Control).
    pub fn is_bulk(self) -> bool {
        !matches!(self, Self::Control)
    }
}

/// Parsed bulk frame header. Stack-allocated.
///
/// Two independent indices:
/// - `nonce`: globally unique per cipher key — used for AEAD and replay detection
/// - `chunk_seq`: per-stream, 0-indexed per transfer — used for reassembly ordering
#[derive(Copy, Clone, Debug)]
pub struct BulkFrameHeader {
    pub stream_id: u8,
    pub kind: FrameKind,
    pub nonce: u64,
    pub chunk_seq: u32,
}

impl BulkFrameHeader {
    pub fn new(stream_id: u8, kind: FrameKind, nonce: u64, chunk_seq: u32) -> Self {
        Self { stream_id, kind, nonce, chunk_seq }
    }

    /// Serialize into a 14-byte array.
    pub fn encode_array(&self) -> [u8; HEADER_LEN] {
        let mut buf = [0u8; HEADER_LEN];
        buf[0] = self.stream_id;
        buf[1] = self.kind as u8;
        buf[2..10].copy_from_slice(&self.nonce.to_le_bytes());
        buf[10..14].copy_from_slice(&self.chunk_seq.to_le_bytes());
        buf
    }

    /// Parse from a byte slice. Returns None if too short or unknown kind.
    pub fn decode(src: &[u8]) -> Option<Self> {
        if src.len() < HEADER_LEN {
            return None;
        }
        let kind = FrameKind::from_byte(src[1])?;
        Some(Self {
            stream_id: src[0],
            kind,
            nonce: u64::from_le_bytes([
                src[2], src[3], src[4], src[5], src[6], src[7], src[8], src[9],
            ]),
            chunk_seq: u32::from_le_bytes([
                src[10], src[11], src[12], src[13],
            ]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let hdr = BulkFrameHeader::new(7, FrameKind::BulkData, 0xDEAD_BEEF_CAFE_BABE, 42);
        let encoded = hdr.encode_array();
        let decoded = BulkFrameHeader::decode(&encoded).unwrap();
        assert_eq!(decoded.stream_id, 7);
        assert_eq!(decoded.kind, FrameKind::BulkData);
        assert_eq!(decoded.nonce, 0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(decoded.chunk_seq, 42);
    }

    #[test]
    fn unknown_kind_returns_none() {
        let mut buf = [0u8; HEADER_LEN];
        buf[1] = 0xFF;
        assert!(BulkFrameHeader::decode(&buf).is_none());
    }
}
