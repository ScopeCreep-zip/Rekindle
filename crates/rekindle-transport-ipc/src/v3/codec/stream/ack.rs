#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAckPayload {
    pub transfer_id: uuid::Uuid,
    pub ack_byte_count: u64,
    pub ack_chunk_count: u32,
    pub audit_link: [u8; 32],
}

pub fn encode(p: &StreamAckPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(p.transfer_id.as_bytes());         // 0..16
    buf.extend_from_slice(&p.ack_byte_count.to_le_bytes());  // 16..24
    buf.extend_from_slice(&p.ack_chunk_count.to_le_bytes()); // 24..28
    buf.extend_from_slice(&[0u8; 4]);                        // 28..32 reserved
    buf.extend_from_slice(&p.audit_link);                    // 32..64
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamAckPayload, CodecError> {
    if buf.len() < 64 {
        return Err(CodecError::TooShort { expected: 64, got: buf.len() });
    }
    let transfer_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let ack_byte_count = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let ack_chunk_count = u32::from_le_bytes(buf[24..28].try_into().unwrap());
    let mut audit_link = [0u8; 32];
    audit_link.copy_from_slice(&buf[32..64]);

    Ok(StreamAckPayload { transfer_id, ack_byte_count, ack_chunk_count, audit_link })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
