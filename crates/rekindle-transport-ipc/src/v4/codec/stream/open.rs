use crate::v4::wire::clearance::Clearance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamOpenPayload {
    pub transfer_id: uuid::Uuid,
    pub expected_total_bytes: u64,
    pub expected_chunk_count: u32,
    pub chunk_size: u32,
    pub content_hash: [u8; 32],
    pub lineage_kind: u8,
    pub dedup_hint: u8,
    pub clearance_required: Clearance,
    pub conditions: Vec<u8>,
}

pub fn encode(p: &StreamOpenPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(72 + p.conditions.len());
    buf.extend_from_slice(p.transfer_id.as_bytes());                  // 0..16
    buf.extend_from_slice(&p.expected_total_bytes.to_le_bytes());     // 16..24
    buf.extend_from_slice(&p.expected_chunk_count.to_le_bytes());     // 24..28
    buf.extend_from_slice(&p.chunk_size.to_le_bytes());               // 28..32
    buf.extend_from_slice(&p.content_hash);                           // 32..64
    buf.push(p.lineage_kind);                                         // 64
    buf.push(p.dedup_hint);                                           // 65
    buf.push(p.clearance_required as u8);                             // 66
    buf.push(0);                                                      // 67 reserved
    crate::v4::codec::write_u32_prefixed(&mut buf, &p.conditions);
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamOpenPayload, CodecError> {
    if buf.len() < 72 {
        return Err(CodecError::TooShort { expected: 72, got: buf.len() });
    }
    let transfer_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let expected_total_bytes = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let expected_chunk_count = u32::from_le_bytes(buf[24..28].try_into().unwrap());
    let chunk_size = u32::from_le_bytes(buf[28..32].try_into().unwrap());
    let mut content_hash = [0u8; 32];
    content_hash.copy_from_slice(&buf[32..64]);
    let lineage_kind = buf[64];
    let dedup_hint = buf[65];
    let clearance_required =
        Clearance::try_from(buf[66]).map_err(|e| CodecError::InvalidClearance(e.0))?;
    let cond_len = u32::from_le_bytes(buf[68..72].try_into().unwrap()) as usize;
    if buf.len() < 72 + cond_len {
        return Err(CodecError::TooShort { expected: 72 + cond_len, got: buf.len() });
    }
    let conditions = buf[72..72 + cond_len].to_vec();

    Ok(StreamOpenPayload {
        transfer_id, expected_total_bytes, expected_chunk_count, chunk_size,
        content_hash, lineage_kind, dedup_hint, clearance_required, conditions,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidClearance(u8),
}
