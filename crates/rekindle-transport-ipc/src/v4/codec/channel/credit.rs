#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditPayload {
    pub scope: u8,
    pub lane_or_stream_id: u8,
    pub credit_bytes: u32,
    pub credit_frames: u32,
    pub credit_generation: u64,
}

pub fn encode(p: &CreditPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24);
    buf.push(p.scope);                                            // 0
    buf.push(p.lane_or_stream_id);                                // 1
    buf.extend_from_slice(&[0u8; 2]);                             // 2..4 reserved
    buf.extend_from_slice(&p.credit_bytes.to_le_bytes());         // 4..8
    buf.extend_from_slice(&p.credit_frames.to_le_bytes());        // 8..12
    buf.extend_from_slice(&[0u8; 4]);                             // 12..16 reserved
    buf.extend_from_slice(&p.credit_generation.to_le_bytes());    // 16..24
    buf
}

pub fn decode(buf: &[u8]) -> Result<CreditPayload, CodecError> {
    if buf.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, got: buf.len() });
    }
    Ok(CreditPayload {
        scope: buf[0],
        lane_or_stream_id: buf[1],
        credit_bytes: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
        credit_frames: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
        credit_generation: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
