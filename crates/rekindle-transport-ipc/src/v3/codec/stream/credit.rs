#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamCreditPayload {
    pub bytes: u32,
    pub chunks: u32,
    pub generation: u64,
}

pub fn encode(p: &StreamCreditPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&p.bytes.to_le_bytes());
    buf.extend_from_slice(&p.chunks.to_le_bytes());
    buf.extend_from_slice(&p.generation.to_le_bytes());
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamCreditPayload, CodecError> {
    if buf.len() < 16 {
        return Err(CodecError::TooShort { expected: 16, got: buf.len() });
    }
    Ok(StreamCreditPayload {
        bytes: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
        chunks: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
        generation: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
