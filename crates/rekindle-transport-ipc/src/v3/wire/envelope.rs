//! EnvelopeWire — the 32-byte outer Packet header.
//!
//! Layout:
//!   offset 0:  wire_version  (u8)
//!   offset 1:  lane          (u8)
//!   offset 2:  flags         (u16 LE)
//!   offset 4:  body_len      (u32 LE)
//!   offset 8:  session_seq   (u64 LE)
//!   offset 16: envelope_mac  ([u8; 16])

use static_assertions::const_assert_eq;

/// On-wire Envelope structure. `#[repr(C)]` guarantees field ordering
/// matches the layout table. Not `packed` — fields are naturally aligned.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct EnvelopeWire {
    pub wire_version: u8,
    pub lane: u8,
    pub flags: u16,
    pub body_len: u32,
    pub session_seq: u64,
    pub envelope_mac: [u8; 16],
}

const_assert_eq!(std::mem::size_of::<EnvelopeWire>(), 32);

/// Byte offsets of each field within EnvelopeWire.
/// Used by codec and tests; no other module should use these.
pub mod offsets {
    pub const WIRE_VERSION: usize = 0;
    pub const LANE: usize = 1;
    pub const FLAGS: usize = 2;
    pub const BODY_LEN: usize = 4;
    pub const SESSION_SEQ: usize = 8;
    pub const ENVELOPE_MAC: usize = 16;

    /// Bytes covered by the EMAC (everything before the MAC field).
    pub const EMAC_INPUT_LEN: usize = 16;
}

/// Envelope flag bits.
pub mod flags {
    pub const SHUTDOWN_INITIATED: u16    = 1 << 0;
    pub const QUIESCENCE_REQUESTED: u16  = 1 << 1;
    pub const BATCH_BOUNDARY: u16        = 1 << 2;
    pub const AUDIT_CHECKPOINT_DUE: u16  = 1 << 3;
    /// Key epoch toggle — 0 = even generation, 1 = odd generation.
    /// Set by the encoder after key rotation. The decoder reads this
    /// bit from the plaintext envelope flags (before EMAC verification)
    /// to select the correct key set. Authenticated by EMAC inclusion.
    pub const KEY_EPOCH: u16             = 1 << 4;

    /// Mask of all defined flag bits. Bits outside this mask are reserved.
    pub const DEFINED_MASK: u16 = 0x001F;

    /// Mask of reserved bits that must be zero.
    pub const RESERVED_MASK: u16 = !DEFINED_MASK;
}
