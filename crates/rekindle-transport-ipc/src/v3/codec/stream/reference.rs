use crate::v3::wire::clearance::Clearance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamReferencePayload {
    pub transfer_id: uuid::Uuid,
    pub content_hash: [u8; 32],
    pub expected_total_bytes: u64,
    pub expected_chunk_count: u32,
    pub reference_kind: u8,
    pub sender_clearance: Clearance,
    pub prefix_byte_count: u64,
    pub prefix_content_hash: [u8; 32],
}

pub fn encode(p: &StreamReferencePayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(104);
    buf.extend_from_slice(p.transfer_id.as_bytes());                  // 0..16
    buf.extend_from_slice(&p.content_hash);                           // 16..48
    buf.extend_from_slice(&p.expected_total_bytes.to_le_bytes());     // 48..56
    buf.extend_from_slice(&p.expected_chunk_count.to_le_bytes());     // 56..60
    buf.push(p.reference_kind);                                       // 60
    buf.push(p.sender_clearance as u8);                               // 61
    buf.extend_from_slice(&[0u8; 2]);                                 // 62..64 reserved
    buf.extend_from_slice(&p.prefix_byte_count.to_le_bytes());        // 64..72
    buf.extend_from_slice(&p.prefix_content_hash);                    // 72..104
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamReferencePayload, CodecError> {
    if buf.len() < 104 {
        return Err(CodecError::TooShort { expected: 104, got: buf.len() });
    }
    let transfer_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let mut content_hash = [0u8; 32];
    content_hash.copy_from_slice(&buf[16..48]);
    let expected_total_bytes = u64::from_le_bytes(buf[48..56].try_into().unwrap());
    let expected_chunk_count = u32::from_le_bytes(buf[56..60].try_into().unwrap());
    let reference_kind = buf[60];
    let sender_clearance = Clearance::try_from(buf[61]).map_err(|e| CodecError::InvalidClearance(e.0))?;
    let prefix_byte_count = u64::from_le_bytes(buf[64..72].try_into().unwrap());
    let mut prefix_content_hash = [0u8; 32];
    prefix_content_hash.copy_from_slice(&buf[72..104]);

    Ok(StreamReferencePayload {
        transfer_id, content_hash, expected_total_bytes, expected_chunk_count,
        reference_kind, sender_clearance, prefix_byte_count, prefix_content_hash,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidClearance(u8),
}
