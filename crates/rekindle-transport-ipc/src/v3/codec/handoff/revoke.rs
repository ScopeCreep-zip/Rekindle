use crate::v3::wire::failure::FailureCode;
use crate::v3::codec::channel::nack::failure_code_from_u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffRevokePayload {
    pub reason_code: FailureCode,
    pub handoff_id: uuid::Uuid,
}

pub fn encode(p: &HandoffRevokePayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(28);
    buf.extend_from_slice(&(p.reason_code as u32).to_le_bytes()); // 0..4
    buf.extend_from_slice(&[0u8; 4]);                             // 4..8 reserved
    buf.extend_from_slice(p.handoff_id.as_bytes());               // 8..24
    buf.extend_from_slice(&[0u8; 4]);                             // 24..28 reserved
    buf
}

pub fn decode(buf: &[u8]) -> Result<HandoffRevokePayload, CodecError> {
    if buf.len() < 28 {
        return Err(CodecError::TooShort { expected: 28, got: buf.len() });
    }
    let reason_u32 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let reason_code = failure_code_from_u32(reason_u32).map_err(|_| CodecError::InvalidFailureCode(reason_u32))?;
    let handoff_id = uuid::Uuid::from_bytes(buf[8..24].try_into().unwrap());

    Ok(HandoffRevokePayload { reason_code, handoff_id })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidFailureCode(u32),
}
