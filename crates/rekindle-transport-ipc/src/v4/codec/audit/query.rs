#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQueryPayload {
    pub query_id: uuid::Uuid,
    pub query_session_seq: u64,
}

pub fn encode(p: &AuditQueryPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32);
    buf.extend_from_slice(p.query_id.as_bytes());                // 0..16
    buf.extend_from_slice(&p.query_session_seq.to_le_bytes());   // 16..24
    buf.extend_from_slice(&[0u8; 8]);                            // 24..32 reserved
    buf
}

pub fn decode(buf: &[u8]) -> Result<AuditQueryPayload, CodecError> {
    if buf.len() < 32 {
        return Err(CodecError::TooShort { expected: 32, got: buf.len() });
    }
    let query_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let query_session_seq = u64::from_le_bytes(buf[16..24].try_into().unwrap());

    Ok(AuditQueryPayload { query_id, query_session_seq })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
