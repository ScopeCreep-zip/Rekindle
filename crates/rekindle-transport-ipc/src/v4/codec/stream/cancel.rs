#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamCancelPayload {
    pub transfer_id: uuid::Uuid,
    pub bytes_through: u64,
    pub chunks_through: u32,
}

pub fn encode(p: &StreamCancelPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32);
    buf.extend_from_slice(p.transfer_id.as_bytes());          // 0..16
    buf.extend_from_slice(&p.bytes_through.to_le_bytes());    // 16..24
    buf.extend_from_slice(&p.chunks_through.to_le_bytes());   // 24..28
    buf.extend_from_slice(&[0u8; 4]);                         // 28..32 reserved
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamCancelPayload, CodecError> {
    if buf.len() < 32 {
        return Err(CodecError::TooShort { expected: 32, got: buf.len() });
    }
    Ok(StreamCancelPayload {
        transfer_id: uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap()),
        bytes_through: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
        chunks_through: u32::from_le_bytes(buf[24..28].try_into().unwrap()),
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
