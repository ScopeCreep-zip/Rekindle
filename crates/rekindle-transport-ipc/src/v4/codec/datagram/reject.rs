use crate::v4::wire::failure::FailureCode;
use crate::v4::codec::channel::nack::failure_code_from_u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatagramRejectPayload {
    pub reason_code: FailureCode,
    pub rejected_message_id: uuid::Uuid,
    pub detail: String,
}

pub fn encode(p: &DatagramRejectPayload) -> Vec<u8> {
    let detail_bytes = p.detail.as_bytes();
    let mut buf = Vec::with_capacity(24 + detail_bytes.len());
    buf.extend_from_slice(&(p.reason_code as u32).to_le_bytes());      // 0..4
    crate::v4::codec::write_u32_len(&mut buf, detail_bytes.len());
    buf.extend_from_slice(p.rejected_message_id.as_bytes());           // 8..24
    buf.extend_from_slice(detail_bytes);                               // 24..
    buf
}

pub fn decode(buf: &[u8]) -> Result<DatagramRejectPayload, CodecError> {
    if buf.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, got: buf.len() });
    }
    let reason_u32 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let reason_code = failure_code_from_u32(reason_u32).map_err(|_| CodecError::InvalidFailureCode(reason_u32))?;
    let detail_len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let rejected_message_id = uuid::Uuid::from_bytes(buf[8..24].try_into().unwrap());
    if buf.len() < 24 + detail_len {
        return Err(CodecError::TooShort { expected: 24 + detail_len, got: buf.len() });
    }
    let detail = String::from_utf8(buf[24..24 + detail_len].to_vec())
        .map_err(|_| CodecError::InvalidUtf8)?;

    Ok(DatagramRejectPayload { reason_code, rejected_message_id, detail })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidUtf8,
    InvalidFailureCode(u32),
}
