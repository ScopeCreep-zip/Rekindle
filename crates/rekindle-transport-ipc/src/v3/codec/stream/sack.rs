#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSackPayload {
    pub transfer_id: uuid::Uuid,
    pub cumulative_through: u32,
    pub sack_bitmap: Vec<u8>,
}

pub fn encode(p: &StreamSackPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + p.sack_bitmap.len());
    buf.extend_from_slice(p.transfer_id.as_bytes());                    // 0..16
    buf.extend_from_slice(&p.cumulative_through.to_le_bytes());
    crate::v3::codec::write_u32_len(&mut buf, p.sack_bitmap.len());
    crate::v3::codec::write_u32_len(&mut buf, p.sack_bitmap.len());
    buf.extend_from_slice(&[0u8; 4]);
    buf.extend_from_slice(&p.sack_bitmap);
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamSackPayload, CodecError> {
    if buf.len() < 32 {
        return Err(CodecError::TooShort { expected: 32, got: buf.len() });
    }
    let transfer_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let cumulative_through = u32::from_le_bytes(buf[16..20].try_into().unwrap());
    let bitmap_len = u32::from_le_bytes(buf[24..28].try_into().unwrap()) as usize;
    if buf.len() < 32 + bitmap_len {
        return Err(CodecError::TooShort { expected: 32 + bitmap_len, got: buf.len() });
    }
    let sack_bitmap = buf[32..32 + bitmap_len].to_vec();

    Ok(StreamSackPayload { transfer_id, cumulative_through, sack_bitmap })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
