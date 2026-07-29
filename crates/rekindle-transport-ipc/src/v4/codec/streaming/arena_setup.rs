//! ArenaSetup encode/decode. Fixed 10 bytes on wire.
//!
//! Sent by the server after arena memfds are delivered via sidechannel.
//! Confirms slot_size, slot_count, and integrity mode so the client can
//! verify fstat sizes match before mapping.

use super::CodecError;

pub const WIRE_SIZE: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaSetupPayload {
    pub slot_size: u32,
    pub slot_count: u16,
    /// 1 = BLAKE3 integrity check on every SharedMemRef. 0 = skip.
    pub integrity: u8,
}

/// Encode an ArenaSetup into a fixed-size buffer.
pub fn encode(setup: &ArenaSetupPayload) -> [u8; WIRE_SIZE] {
    let mut buf = [0u8; WIRE_SIZE];
    buf[0..4].copy_from_slice(&setup.slot_size.to_le_bytes());
    buf[4..6].copy_from_slice(&setup.slot_count.to_le_bytes());
    buf[6] = setup.integrity;
    // buf[7..10] reserved, zero-filled by array init
    buf
}

/// Decode an ArenaSetup from `buf`.
pub fn decode(buf: &[u8]) -> Result<ArenaSetupPayload, CodecError> {
    if buf.len() < WIRE_SIZE {
        return Err(CodecError::TooShort {
            expected: WIRE_SIZE,
            got: buf.len(),
        });
    }

    Ok(ArenaSetupPayload {
        slot_size: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
        slot_count: u16::from_le_bytes([buf[4], buf[5]]),
        integrity: buf[6],
    })
}
