#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditProofPayload {
    pub query_id: uuid::Uuid,
    pub result_session_seq: u64,
    pub link: [u8; 32],
    pub anchor_link: [u8; 32],
    pub segment: Vec<[u8; 32]>,
}

pub fn encode(p: &AuditProofPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96 + p.segment.len() * 32);
    buf.extend_from_slice(p.query_id.as_bytes());                     // 0..16
    buf.extend_from_slice(&p.result_session_seq.to_le_bytes());       // 16..24
    crate::v3::codec::write_u32_len(&mut buf, p.segment.len());
    buf.extend_from_slice(&[0u8; 4]);                                 // 28..32 reserved
    buf.extend_from_slice(&p.link);                                   // 32..64
    buf.extend_from_slice(&p.anchor_link);                            // 64..96
    for link in &p.segment {
        buf.extend_from_slice(link);
    }
    buf
}

pub fn decode(buf: &[u8]) -> Result<AuditProofPayload, CodecError> {
    if buf.len() < 96 {
        return Err(CodecError::TooShort { expected: 96, got: buf.len() });
    }
    let query_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let result_session_seq = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let segment_len = u32::from_le_bytes(buf[24..28].try_into().unwrap()) as usize;
    let mut link = [0u8; 32];
    link.copy_from_slice(&buf[32..64]);
    let mut anchor_link = [0u8; 32];
    anchor_link.copy_from_slice(&buf[64..96]);

    let total = 96 + segment_len * 32;
    if buf.len() < total {
        return Err(CodecError::TooShort { expected: total, got: buf.len() });
    }
    let mut segment = Vec::with_capacity(segment_len);
    for i in 0..segment_len {
        let start = 96 + i * 32;
        let mut entry = [0u8; 32];
        entry.copy_from_slice(&buf[start..start + 32]);
        segment.push(entry);
    }

    Ok(AuditProofPayload { query_id, result_session_seq, link, anchor_link, segment })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
