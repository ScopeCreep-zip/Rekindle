//! Wire frame format for the bulk transfer plane.
//!
//! Every bulk frame on the wire is structured as:
//!
//! ```text
//! [1B lane byte] ← written/read separately by lane protocol
//! [4B length BE] ← written/read by write_frame/read_frame (framing layer)
//! [1B stream_id] ← bulk header start (this module)
//! [1B kind]
//! [8B nonce LE]
//! [N bytes ciphertext]
//! [16B AEAD tag]
//! ```
//!
//! The lane byte and length prefix are handled by the server's lane
//! protocol and the framing layer respectively. This module defines
//! the bulk header (stream_id + kind + nonce) that sits INSIDE the
//! length-prefixed frame body.

/// Bulk header length: stream_id(1) + kind(1) + nonce(8) = 10 bytes.
///
/// This does NOT include the 4-byte length prefix (handled by the
/// framing layer) or the 1-byte lane byte (handled by the lane protocol).
pub const HEADER_LEN: usize = 10;

/// AES-256-GCM authentication tag length.
pub const TAG_LEN: usize = 16;

/// Maximum plaintext per bulk chunk. Matches the Noise MAXMSGLEN - TAGLEN
/// (65535 - 16 = 65519) for consistency.
pub const MAX_CHUNK_PLAIN: usize = 65_519;

/// Maximum frame body size: header + max ciphertext + tag.
pub const MAX_FRAME_BODY: usize = HEADER_LEN + MAX_CHUNK_PLAIN + TAG_LEN;

/// Discriminant for the 1-byte lane prefix and the kind field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    /// Noise-encrypted control-plane message (request/response/event).
    Control = 0x00,
    /// Bulk transfer data chunk.
    BulkData = 0x01,
    /// Final chunk of a bulk transfer (carries blob digest).
    BulkFin = 0x02,
    /// Flow control: receiver grants send credit.
    WindowUpdate = 0x03,
}

impl FrameKind {
    /// Parse a frame kind from a single byte.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Control),
            0x01 => Some(Self::BulkData),
            0x02 => Some(Self::BulkFin),
            0x03 => Some(Self::WindowUpdate),
            _ => None,
        }
    }

    /// Returns `true` for bulk-plane frame kinds (not Control).
    pub fn is_bulk(self) -> bool {
        !matches!(self, Self::Control)
    }
}

/// Parsed bulk frame header. Stack-allocated.
///
/// Represents the 10-byte sub-header inside the frame body:
/// `[stream_id(1)][kind(1)][nonce(8)]`.
///
/// The 4-byte length prefix is NOT part of this header — it is
/// handled by the framing layer (`write_frame`/`read_frame`).
#[derive(Copy, Clone, Debug)]
pub struct BulkFrameHeader {
    /// Logical stream identifier (0–255).
    pub stream_id: u8,
    /// Frame type discriminant.
    pub kind: FrameKind,
    /// AEAD nonce counter (LE u64).
    pub nonce: u64,
}

impl BulkFrameHeader {
    /// Construct a header for a data or fin frame.
    pub fn new(stream_id: u8, kind: FrameKind, nonce: u64) -> Self {
        Self { stream_id, kind, nonce }
    }

    /// Serialize the header into a 10-byte buffer.
    pub fn encode(&self, out: &mut [u8; HEADER_LEN]) {
        out[0] = self.stream_id;
        out[1] = self.kind as u8;
        out[2..10].copy_from_slice(&self.nonce.to_le_bytes());
    }

    /// Encode into a new array.
    pub fn encode_array(&self) -> [u8; HEADER_LEN] {
        let mut buf = [0u8; HEADER_LEN];
        self.encode(&mut buf);
        buf
    }

    /// Parse a header from a byte slice.
    /// Returns `None` if the slice is too short or the kind byte is unknown.
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
        })
    }

    /// Compute the ciphertext length from a total frame body length.
    ///
    /// `body_len` is the number of bytes in the frame body (after the
    /// 4-byte length prefix, as returned by `read_frame`).
    /// Returns `None` if body_len is too small for header + tag.
    pub fn ciphertext_len_from_body(body_len: usize) -> Option<usize> {
        let overhead = HEADER_LEN + TAG_LEN; // 26
        if body_len < overhead {
            return None;
        }
        Some(body_len - overhead)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_header() {
        let hdr = BulkFrameHeader::new(7, FrameKind::BulkData, 0xDEAD_BEEF_CAFE_BABE);
        let encoded = hdr.encode_array();
        assert_eq!(encoded.len(), HEADER_LEN);

        let decoded = BulkFrameHeader::decode(&encoded).expect("decode failed");
        assert_eq!(decoded.stream_id, 7);
        assert_eq!(decoded.kind, FrameKind::BulkData);
        assert_eq!(decoded.nonce, 0xDEAD_BEEF_CAFE_BABE);
    }

    #[test]
    fn ciphertext_len_calculation() {
        let body_len = HEADER_LEN + 1024 + TAG_LEN;
        assert_eq!(BulkFrameHeader::ciphertext_len_from_body(body_len), Some(1024));
    }

    #[test]
    fn too_short_body_returns_none() {
        assert_eq!(BulkFrameHeader::ciphertext_len_from_body(25), None);
    }

    #[test]
    fn unknown_kind_byte_returns_none() {
        let mut buf = [0u8; HEADER_LEN];
        buf[0] = 0; // stream_id
        buf[1] = 0xFF; // unknown kind
        assert!(BulkFrameHeader::decode(&buf).is_none());
    }

    #[test]
    fn too_short_slice_returns_none() {
        assert!(BulkFrameHeader::decode(&[0u8; 9]).is_none());
    }

    #[test]
    fn frame_kind_is_bulk() {
        assert!(!FrameKind::Control.is_bulk());
        assert!(FrameKind::BulkData.is_bulk());
        assert!(FrameKind::BulkFin.is_bulk());
        assert!(FrameKind::WindowUpdate.is_bulk());
    }

    #[test]
    fn encode_array_matches_encode() {
        let hdr = BulkFrameHeader::new(42, FrameKind::BulkFin, 999);
        let arr = hdr.encode_array();
        let mut buf = [0u8; HEADER_LEN];
        hdr.encode(&mut buf);
        assert_eq!(arr, buf);
    }
}
