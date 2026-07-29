//! SlotRelease encode/decode. Fixed 7 bytes on wire.

use crate::v4::streaming::SlotRelease;
use super::CodecError;

pub const WIRE_SIZE: usize = 7;

/// Encode a SlotRelease into a fixed-size buffer.
#[inline]
pub fn encode(release: &SlotRelease, buf: &mut [u8]) {
    assert!(buf.len() >= WIRE_SIZE, "slot_release::encode: buf too short");
    buf[0] = release.arena_id;
    buf[1..3].copy_from_slice(&release.slot.to_le_bytes());
    buf[3..7].copy_from_slice(&release.generation.to_le_bytes());
}

/// Decode a SlotRelease from `buf`.
#[inline]
pub fn decode(buf: &[u8]) -> Result<SlotRelease, CodecError> {
    if buf.len() < WIRE_SIZE {
        return Err(CodecError::TooShort {
            expected: WIRE_SIZE,
            got: buf.len(),
        });
    }

    Ok(SlotRelease {
        arena_id: buf[0],
        slot: u16::from_le_bytes([buf[1], buf[2]]),
        generation: u32::from_le_bytes(buf[3..7].try_into().unwrap()),
    })
}
