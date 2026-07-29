//! SharedMemRef encode/decode.
//!
//! Variable-length wire format:
//! - 15 bytes when integrity=0 (production fast path)
//! - 47 bytes when integrity=1 (debug/validation, includes BLAKE3 digest)
//!
//! The integrity flag is negotiated per-connection in ArenaSetup and known
//! to both sides at codec time — no per-message ambiguity.

use crate::v4::streaming::SharedMemRef;
use super::CodecError;

/// Wire size without BLAKE3 digest.
pub const WIRE_SIZE_NO_DIGEST: usize = 15;
/// Wire size with BLAKE3 digest.
pub const WIRE_SIZE_WITH_DIGEST: usize = 47;

/// Returns the wire size for the current integrity mode.
pub const fn wire_size(integrity: bool) -> usize {
    if integrity {
        WIRE_SIZE_WITH_DIGEST
    } else {
        WIRE_SIZE_NO_DIGEST
    }
}

/// Encode a SharedMemRef into `buf`.
///
/// `buf` must be at least `wire_size(integrity)` bytes.
///
/// # Panics
///
/// Panics if `buf` is too short.
#[inline]
pub fn encode(shmref: &SharedMemRef, buf: &mut [u8], integrity: bool) {
    let required = wire_size(integrity);
    assert!(
        buf.len() >= required,
        "arena_write::encode: buf.len() {} < required {required}",
        buf.len(),
    );

    buf[0] = shmref.arena_id;
    buf[1..3].copy_from_slice(&shmref.slot.to_le_bytes());
    buf[3..7].copy_from_slice(&shmref.generation.to_le_bytes());
    buf[7..11].copy_from_slice(&shmref.offset.to_le_bytes());
    buf[11..15].copy_from_slice(&shmref.length.to_le_bytes());
    if integrity {
        let digest = shmref.digest.unwrap_or([0u8; 32]);
        buf[15..47].copy_from_slice(&digest);
    }
}

/// Decode a SharedMemRef from `buf`.
///
/// `integrity` must match the connection's negotiated setting.
#[inline]
pub fn decode(buf: &[u8], integrity: bool) -> Result<SharedMemRef, CodecError> {
    let expected = wire_size(integrity);
    if buf.len() < expected {
        return Err(CodecError::TooShort {
            expected,
            got: buf.len(),
        });
    }

    Ok(SharedMemRef {
        arena_id: buf[0],
        slot: u16::from_le_bytes([buf[1], buf[2]]),
        generation: u32::from_le_bytes(buf[3..7].try_into().unwrap()),
        offset: u32::from_le_bytes(buf[7..11].try_into().unwrap()),
        length: u32::from_le_bytes(buf[11..15].try_into().unwrap()),
        digest: if integrity {
            let mut d = [0u8; 32];
            d.copy_from_slice(&buf[15..47]);
            Some(d)
        } else {
            None
        },
    })
}
