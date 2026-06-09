#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffConfirmPayload {
    pub handoff_id: uuid::Uuid,
}

pub fn encode(p: &HandoffConfirmPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&[0u8; 4]);                    // 0..4 reason_code=0 (success)
    buf.extend_from_slice(&[0u8; 4]);                    // 4..8 reserved
    buf.extend_from_slice(p.handoff_id.as_bytes());      // 8..24
    buf
}

pub fn decode(buf: &[u8]) -> Result<HandoffConfirmPayload, CodecError> {
    if buf.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, got: buf.len() });
    }
    let handoff_id = uuid::Uuid::from_bytes(buf[8..24].try_into().unwrap());
    Ok(HandoffConfirmPayload { handoff_id })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
