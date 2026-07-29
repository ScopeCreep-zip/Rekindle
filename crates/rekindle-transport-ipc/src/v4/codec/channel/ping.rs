#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingPayload {
    pub ping_nonce: u64,
    pub sender_epoch_ns: u64,
    pub last_seen_remote_seq: u64,
}

pub fn encode(p: &PingPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&p.ping_nonce.to_le_bytes());
    buf.extend_from_slice(&p.sender_epoch_ns.to_le_bytes());
    buf.extend_from_slice(&p.last_seen_remote_seq.to_le_bytes());
    buf
}

pub fn decode(buf: &[u8]) -> Result<PingPayload, CodecError> {
    if buf.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, got: buf.len() });
    }
    Ok(PingPayload {
        ping_nonce: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
        sender_epoch_ns: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
        last_seen_remote_seq: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
