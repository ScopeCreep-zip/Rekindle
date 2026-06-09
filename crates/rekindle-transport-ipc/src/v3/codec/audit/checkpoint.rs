#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditCheckpointPayload {
    pub chain_index: u64,
    pub chain_length: u64,
    pub checkpoint_seq: u64,
    pub wall_clock_ns: u64,
    pub chain_link: [u8; 32],
    pub anchor_link: [u8; 32],
}

pub fn encode(p: &AuditCheckpointPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(&p.chain_index.to_le_bytes());     // 0..8
    buf.extend_from_slice(&p.chain_length.to_le_bytes());    // 8..16
    buf.extend_from_slice(&p.checkpoint_seq.to_le_bytes());  // 16..24
    buf.extend_from_slice(&p.wall_clock_ns.to_le_bytes());   // 24..32
    buf.extend_from_slice(&p.chain_link);                    // 32..64
    buf.extend_from_slice(&p.anchor_link);                   // 64..96
    buf
}

pub fn decode(buf: &[u8]) -> Result<AuditCheckpointPayload, CodecError> {
    if buf.len() < 96 {
        return Err(CodecError::TooShort { expected: 96, got: buf.len() });
    }
    let chain_index = u64::from_le_bytes(buf[0..8].try_into().unwrap());
    let chain_length = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    let checkpoint_seq = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let wall_clock_ns = u64::from_le_bytes(buf[24..32].try_into().unwrap());
    let mut chain_link = [0u8; 32];
    chain_link.copy_from_slice(&buf[32..64]);
    let mut anchor_link = [0u8; 32];
    anchor_link.copy_from_slice(&buf[64..96]);

    Ok(AuditCheckpointPayload { chain_index, chain_length, checkpoint_seq, wall_clock_ns, chain_link, anchor_link })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
