#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodbyePayload {
    pub reason_code: u32,
    pub drain_timeout_ms: u32,
    pub final_session_seq: u64,
}

pub fn encode(p: &GoodbyePayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&p.reason_code.to_le_bytes());
    buf.extend_from_slice(&p.drain_timeout_ms.to_le_bytes());
    buf.extend_from_slice(&p.final_session_seq.to_le_bytes());
    buf
}

pub fn decode(buf: &[u8]) -> Result<GoodbyePayload, CodecError> {
    if buf.len() < 16 {
        return Err(CodecError::TooShort { expected: 16, got: buf.len() });
    }
    Ok(GoodbyePayload {
        reason_code: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
        drain_timeout_ms: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
        final_session_seq: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
