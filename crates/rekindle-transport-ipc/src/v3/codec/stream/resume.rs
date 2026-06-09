#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamResumePayload {
    pub transfer_id: uuid::Uuid,
    pub resume_from_byte: u64,
    pub resume_from_chunk: u32,
    pub anchor_audit_link: [u8; 32],
    pub anchor_content_hash: [u8; 32],
}

pub fn encode(p: &StreamResumePayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(p.transfer_id.as_bytes());            // 0..16
    buf.extend_from_slice(&p.resume_from_byte.to_le_bytes());   // 16..24
    buf.extend_from_slice(&p.resume_from_chunk.to_le_bytes());  // 24..28
    buf.extend_from_slice(&[0u8; 4]);                           // 28..32 reserved
    buf.extend_from_slice(&p.anchor_audit_link);                // 32..64
    buf.extend_from_slice(&p.anchor_content_hash);              // 64..96
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamResumePayload, CodecError> {
    if buf.len() < 96 {
        return Err(CodecError::TooShort { expected: 96, got: buf.len() });
    }
    let transfer_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let resume_from_byte = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let resume_from_chunk = u32::from_le_bytes(buf[24..28].try_into().unwrap());
    let mut anchor_audit_link = [0u8; 32];
    anchor_audit_link.copy_from_slice(&buf[32..64]);
    let mut anchor_content_hash = [0u8; 32];
    anchor_content_hash.copy_from_slice(&buf[64..96]);

    Ok(StreamResumePayload { transfer_id, resume_from_byte, resume_from_chunk, anchor_audit_link, anchor_content_hash })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
