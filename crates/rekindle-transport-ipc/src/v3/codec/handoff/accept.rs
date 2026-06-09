#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffAcceptPayload {
    pub handoff_id: uuid::Uuid,
    pub accept_wall_ns: u64,
    pub verified_content_hash: [u8; 32],
}

pub fn encode(p: &HandoffAcceptPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(p.handoff_id.as_bytes());                // 0..16
    buf.extend_from_slice(&p.accept_wall_ns.to_le_bytes());       // 16..24
    buf.extend_from_slice(&[0u8; 8]);                              // 24..32 reserved
    buf.extend_from_slice(&p.verified_content_hash);               // 32..64
    buf
}

pub fn decode(buf: &[u8]) -> Result<HandoffAcceptPayload, CodecError> {
    if buf.len() < 64 {
        return Err(CodecError::TooShort { expected: 64, got: buf.len() });
    }
    let handoff_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let accept_wall_ns = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let mut verified_content_hash = [0u8; 32];
    verified_content_hash.copy_from_slice(&buf[32..64]);

    Ok(HandoffAcceptPayload { handoff_id, accept_wall_ns, verified_content_hash })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
