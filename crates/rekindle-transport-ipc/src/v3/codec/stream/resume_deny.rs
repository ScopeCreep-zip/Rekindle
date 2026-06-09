use crate::v3::wire::failure::FailureCode;
use crate::v3::codec::channel::nack::failure_code_from_u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamResumeDenyPayload {
    pub transfer_id: uuid::Uuid,
    pub reason_code: FailureCode,
}

pub fn encode(p: &StreamResumeDenyPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(p.transfer_id.as_bytes());              // 0..16
    buf.extend_from_slice(&(p.reason_code as u32).to_le_bytes()); // 16..20
    buf.extend_from_slice(&[0u8; 4]);                             // 20..24 reserved
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamResumeDenyPayload, CodecError> {
    if buf.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, got: buf.len() });
    }
    let transfer_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let reason_u32 = u32::from_le_bytes(buf[16..20].try_into().unwrap());
    let reason_code = failure_code_from_u32(reason_u32).map_err(|_| CodecError::InvalidFailureCode(reason_u32))?;

    Ok(StreamResumeDenyPayload { transfer_id, reason_code })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidFailureCode(u32),
}
