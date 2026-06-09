//! StreamHeaderWire — the 32-byte inner header for Data Lane frames.
//!
//! Layout:
//!   offset 0:  frame_class   (u8, always 0x02)
//!   offset 1:  frame_kind    (u8)
//!   offset 2:  stream_id     (u8)
//!   offset 3:  header_flags  (u8)
//!   offset 4:  chunk_index   (u32 LE)
//!   offset 8:  nonce         (u64 LE)
//!   offset 16: header_mac    ([u8; 16])

use static_assertions::const_assert_eq;

/// On-wire Stream Header structure.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StreamHeaderWire {
    pub frame_class: u8,
    pub frame_kind: u8,
    pub stream_id: u8,
    pub header_flags: u8,
    pub chunk_index: u32,
    pub nonce: u64,
    pub header_mac: [u8; 16],
}

const_assert_eq!(std::mem::size_of::<StreamHeaderWire>(), 32);

/// Byte offsets within StreamHeaderWire.
pub mod offsets {
    pub const FRAME_CLASS: usize = 0;
    pub const FRAME_KIND: usize = 1;
    pub const STREAM_ID: usize = 2;
    pub const HEADER_FLAGS: usize = 3;
    pub const CHUNK_INDEX: usize = 4;
    pub const NONCE: usize = 8;
    pub const HEADER_MAC: usize = 16;

    /// Bytes covered by the HeaderMAC (everything before the MAC field).
    pub const MAC_INPUT_LEN: usize = 16;
}

/// Stream header flag bits.
pub mod flags {
    pub const FIN_FOLLOWS: u8    = 1 << 0;
    pub const INTEGRITY_HASH: u8 = 1 << 1;
    pub const COMPRESSED: u8     = 1 << 2;
    pub const PRIORITY: u8       = 1 << 3;
    pub const LAST_OF_BATCH: u8  = 1 << 4;

    /// Mask of all defined flag bits.
    pub const DEFINED_MASK: u8 = 0x1F;

    /// Reserved bits that must be zero.
    pub const RESERVED_MASK: u8 = !DEFINED_MASK;
}
