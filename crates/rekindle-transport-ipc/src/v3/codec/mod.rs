//! Layer 2: Codec — encode/decode between wire bytes and domain types.
//!
//! Every byte offset lives here or in `wire/`. No other module
//! reads or writes raw bytes at numeric offsets.

pub mod envelope;
pub mod header;
pub mod aead;
pub mod channel;
pub mod stream;
pub mod datagram;
pub mod audit;
pub mod handoff;

// ── Wire-safe length encoding helpers ─────────────────────────────
//
// Every variable-length field on the wire is prefixed by a u32 or u16
// length. These helpers write the length-prefixed form in one call.
// The length conversion and bounds check happen here — nowhere else.

/// Append a u32-LE length prefix followed by the data to `buf`.
#[inline]
pub(crate) fn write_u32_prefixed(buf: &mut Vec<u8>, data: &[u8]) {
    let len = u32::try_from(data.len())
        .expect("payload exceeds wire format u32 length field maximum");
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(data);
}

/// Append a u32-LE length value (without trailing data) to `buf`.
#[inline]
pub(crate) fn write_u32_len(buf: &mut Vec<u8>, len: usize) {
    let val = u32::try_from(len)
        .expect("length exceeds wire format u32 maximum");
    buf.extend_from_slice(&val.to_le_bytes());
}

/// Append a u16-LE length value (without trailing data) to `buf`.
#[inline]
pub(crate) fn write_u16_len(buf: &mut Vec<u8>, len: usize) {
    let val = u16::try_from(len)
        .expect("length exceeds wire format u16 maximum");
    buf.extend_from_slice(&val.to_le_bytes());
}
