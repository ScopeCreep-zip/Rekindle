use crate::v4::wire::failure::FailureCode;
use crate::v4::codec::channel::nack::failure_code_from_u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamNackPayload {
    pub reason_code: FailureCode,
    pub rejected_chunk_idx: u32,
    pub detail: String,
}

pub fn encode(p: &StreamNackPayload) -> Vec<u8> {
    let detail_bytes = p.detail.as_bytes();
    let mut buf = Vec::with_capacity(16 + detail_bytes.len());
    buf.extend_from_slice(&(p.reason_code as u32).to_le_bytes());
    buf.extend_from_slice(&p.rejected_chunk_idx.to_le_bytes());
    crate::v4::codec::write_u32_len(&mut buf, detail_bytes.len());
    buf.extend_from_slice(&[0u8; 4]);
    buf.extend_from_slice(detail_bytes);
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamNackPayload, CodecError> {
    if buf.len() < 16 {
        return Err(CodecError::TooShort { expected: 16, got: buf.len() });
    }
    let reason_u32 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let reason_code = failure_code_from_u32(reason_u32).map_err(|_| CodecError::InvalidFailureCode(reason_u32))?;
    let rejected_chunk_idx = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    let detail_len = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
    if buf.len() < 16 + detail_len {
        return Err(CodecError::TooShort { expected: 16 + detail_len, got: buf.len() });
    }
    let detail = String::from_utf8(buf[16..16 + detail_len].to_vec())
        .map_err(|_| CodecError::InvalidUtf8)?;

    Ok(StreamNackPayload { reason_code, rejected_chunk_idx, detail })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidUtf8,
    InvalidFailureCode(u32),
}
