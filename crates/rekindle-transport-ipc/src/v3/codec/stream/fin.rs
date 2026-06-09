#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFinPayload {
    pub total_bytes: u64,
    pub fault_count: u32,
    pub final_content_hash: [u8; 32],
    pub final_audit_link: [u8; 32],
}

pub fn encode(p: &StreamFinPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(80);
    buf.extend_from_slice(&p.total_bytes.to_le_bytes());       // 0..8
    buf.extend_from_slice(&p.fault_count.to_le_bytes());       // 8..12
    buf.extend_from_slice(&[0u8; 4]);                          // 12..16 reserved
    buf.extend_from_slice(&p.final_content_hash);              // 16..48
    buf.extend_from_slice(&p.final_audit_link);                // 48..80
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamFinPayload, CodecError> {
    if buf.len() < 80 {
        return Err(CodecError::TooShort { expected: 80, got: buf.len() });
    }
    let total_bytes = u64::from_le_bytes(buf[0..8].try_into().unwrap());
    let fault_count = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    let mut final_content_hash = [0u8; 32];
    final_content_hash.copy_from_slice(&buf[16..48]);
    let mut final_audit_link = [0u8; 32];
    final_audit_link.copy_from_slice(&buf[48..80]);

    Ok(StreamFinPayload { total_bytes, fault_count, final_content_hash, final_audit_link })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
