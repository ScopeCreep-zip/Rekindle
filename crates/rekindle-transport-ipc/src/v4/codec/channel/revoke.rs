//! CHANNEL_REVOKE codec.
//!
//! Layout (48 bytes):
//!   [0]:      artifact_kind (u8)
//!   [1..4]:   reserved (3 bytes, zero)
//!   [4..8]:   reason_code (u32 LE)
//!   [8..40]:  artifact_id ([u8; 32] — UUIDs in first 16, content hashes use all 32)
//!   [40..48]: revoke_generation (u64 LE)

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokePayload {
    pub artifact_kind: u8,
    pub reason_code: u32,
    pub artifact_id: [u8; 32],
    pub revoke_generation: u64,
}

pub fn encode(payload: &RevokePayload) -> Vec<u8> {
    let mut buf = vec![0u8; 48];
    buf[0] = payload.artifact_kind;
    // [1..4] reserved, already zero
    buf[4..8].copy_from_slice(&payload.reason_code.to_le_bytes());
    buf[8..40].copy_from_slice(&payload.artifact_id);
    buf[40..48].copy_from_slice(&payload.revoke_generation.to_le_bytes());
    buf
}

pub fn decode(data: &[u8]) -> Result<RevokePayload, CodecError> {
    if data.len() < 48 {
        return Err(CodecError::TooShort { expected: 48, actual: data.len() });
    }
    let mut artifact_id = [0u8; 32];
    artifact_id.copy_from_slice(&data[8..40]);
    Ok(RevokePayload {
        artifact_kind: data[0],
        reason_code: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        artifact_id,
        revoke_generation: u64::from_le_bytes(data[40..48].try_into().unwrap()),
    })
}
