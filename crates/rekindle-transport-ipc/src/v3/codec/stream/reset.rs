use crate::v3::wire::failure::FailureCode;
use crate::v3::codec::channel::nack::failure_code_from_u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamResetPayload {
    pub reason_code: FailureCode,
}

pub fn encode(p: &StreamResetPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    buf.extend_from_slice(&(p.reason_code as u32).to_le_bytes());
    buf.extend_from_slice(&[0u8; 4]);
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamResetPayload, CodecError> {
    if buf.len() < 8 {
        return Err(CodecError::TooShort { expected: 8, got: buf.len() });
    }
    let reason_u32 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let reason_code = failure_code_from_u32(reason_u32).map_err(|_| CodecError::InvalidFailureCode(reason_u32))?;
    Ok(StreamResetPayload { reason_code })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidFailureCode(u32),
}
