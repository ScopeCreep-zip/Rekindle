//! Stream header codec — serialize/deserialize the 32-byte inner header.
//!
//! HeaderMAC (bytes 16..32) is a BLAKE3-keyed MAC over bytes 0..16.
//! Verified before any routing field is consulted.

use crate::v4::wire::constants::{STREAM_HEADER_LEN, HEADER_MAC_LEN};
use crate::v4::wire::frame_class::FrameClass;
use crate::v4::wire::frame_kind::StreamKind;
use crate::v4::wire::header::{offsets, flags};

/// Domain-level stream header fields extracted after HeaderMAC verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamHeaderInfo {
    pub frame_class: FrameClass,
    pub frame_kind: StreamKind,
    pub stream_id: u8,
    pub header_flags: u8,
    pub chunk_index: u32,
    pub nonce: u64,
}

#[derive(Debug)]
pub enum HeaderError {
    MacFailed,
    FrameClassInvalid(u8),
    FrameKindInvalid(u8),
    ReservedBitSet(u8),
}

/// Build a 32-byte stream header with HeaderMAC.
pub fn build_header(info: &StreamHeaderInfo, header_key: &[u8; 32]) -> [u8; STREAM_HEADER_LEN] {
    let mut buf = [0u8; STREAM_HEADER_LEN];

    buf[offsets::FRAME_CLASS] = info.frame_class as u8;
    buf[offsets::FRAME_KIND] = info.frame_kind as u8;
    buf[offsets::STREAM_ID] = info.stream_id;
    buf[offsets::HEADER_FLAGS] = info.header_flags;
    buf[offsets::CHUNK_INDEX..offsets::CHUNK_INDEX + 4]
        .copy_from_slice(&info.chunk_index.to_le_bytes());
    buf[offsets::NONCE..offsets::NONCE + 8].copy_from_slice(&info.nonce.to_le_bytes());

    let mac = compute_header_mac(&buf[..offsets::MAC_INPUT_LEN], header_key);
    buf[offsets::HEADER_MAC..offsets::HEADER_MAC + HEADER_MAC_LEN].copy_from_slice(&mac);

    buf
}

/// Parse and verify a 32-byte stream header. HeaderMAC checked first.
pub fn parse_header(
    buf: &[u8; STREAM_HEADER_LEN],
    header_key: &[u8; 32],
) -> Result<StreamHeaderInfo, HeaderError> {
    let expected_mac = compute_header_mac(&buf[..offsets::MAC_INPUT_LEN], header_key);
    let actual_mac = &buf[offsets::HEADER_MAC..offsets::HEADER_MAC + HEADER_MAC_LEN];
    if !constant_time_eq(&expected_mac, actual_mac) {
        return Err(HeaderError::MacFailed);
    }

    let frame_class = FrameClass::try_from(buf[offsets::FRAME_CLASS])
        .map_err(|e| HeaderError::FrameClassInvalid(e.0))?;

    let frame_kind = StreamKind::try_from(buf[offsets::FRAME_KIND])
        .map_err(|e| HeaderError::FrameKindInvalid(e.0))?;

    let header_flags = buf[offsets::HEADER_FLAGS];
    if header_flags & flags::RESERVED_MASK != 0 {
        return Err(HeaderError::ReservedBitSet(header_flags));
    }

    let chunk_index = u32::from_le_bytes([
        buf[offsets::CHUNK_INDEX],
        buf[offsets::CHUNK_INDEX + 1],
        buf[offsets::CHUNK_INDEX + 2],
        buf[offsets::CHUNK_INDEX + 3],
    ]);

    let nonce = u64::from_le_bytes([
        buf[offsets::NONCE],
        buf[offsets::NONCE + 1],
        buf[offsets::NONCE + 2],
        buf[offsets::NONCE + 3],
        buf[offsets::NONCE + 4],
        buf[offsets::NONCE + 5],
        buf[offsets::NONCE + 6],
        buf[offsets::NONCE + 7],
    ]);

    Ok(StreamHeaderInfo {
        frame_class,
        frame_kind,
        stream_id: buf[offsets::STREAM_ID],
        header_flags,
        chunk_index,
        nonce,
    })
}

impl StreamHeaderInfo {
    /// Whether the FIN_FOLLOWS flag is set — signals this is the last
    /// chunk and the receiver should verify + ACK immediately without
    /// waiting for a separate STREAM_FIN frame.
    pub fn fin_follows(&self) -> bool {
        self.header_flags & flags::FIN_FOLLOWS != 0
    }
}

fn compute_header_mac(input: &[u8], key: &[u8; 32]) -> [u8; HEADER_MAC_LEN] {
    let hash = blake3::keyed_hash(key, input);
    let mut mac = [0u8; HEADER_MAC_LEN];
    mac.copy_from_slice(&hash.as_bytes()[..HEADER_MAC_LEN]);
    mac
}

fn constant_time_eq(a: &[u8; HEADER_MAC_LEN], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}
