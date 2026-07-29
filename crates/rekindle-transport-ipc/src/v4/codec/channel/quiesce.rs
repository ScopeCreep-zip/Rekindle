//! CHANNEL_QUIESCE / QUIESCE_ACK / RESUME / RESUME_ACK codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuiescePayload {
    pub duration_ms: u64,
    pub reason_code: u32,
}

pub fn decode(data: &[u8]) -> Result<QuiescePayload, CodecError> {
    if data.len() < 16 {
        return Err(CodecError::TooShort { expected: 16, actual: data.len() });
    }
    Ok(QuiescePayload {
        duration_ms: u64::from_le_bytes(data[0..8].try_into().unwrap()),
        reason_code: u32::from_le_bytes(data[8..12].try_into().unwrap()),
    })
}

pub fn encode(p: &QuiescePayload) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0..8].copy_from_slice(&p.duration_ms.to_le_bytes());
    buf[8..12].copy_from_slice(&p.reason_code.to_le_bytes());
    buf
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
}
