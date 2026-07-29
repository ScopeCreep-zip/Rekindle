//! ArenaAck encode/decode. Fixed 1 byte on wire.
//!
//! Sent by the client after receiving ArenaSetup and successfully
//! mapping the arena. STATUS_OK = proceed with streaming.
//! STATUS_REJECTED = fall back to Tier 1 (inline only).

use super::CodecError;

pub const WIRE_SIZE: usize = 1;

pub const STATUS_OK: u8 = 0;
pub const STATUS_REJECTED: u8 = 1;

/// Encode an ArenaAck status into a fixed-size buffer.
pub fn encode(status: u8) -> [u8; WIRE_SIZE] {
    [status]
}

/// Decode an ArenaAck status from `buf`.
pub fn decode(buf: &[u8]) -> Result<u8, CodecError> {
    if buf.is_empty() {
        return Err(CodecError::TooShort {
            expected: WIRE_SIZE,
            got: 0,
        });
    }
    Ok(buf[0])
}
