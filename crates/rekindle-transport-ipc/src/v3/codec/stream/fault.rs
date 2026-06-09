#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFaultPayload {
    pub fault_code: u32,
    pub fault_payload: Vec<u8>,
}

pub fn encode(p: &StreamFaultPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + p.fault_payload.len());
    buf.extend_from_slice(&p.fault_code.to_le_bytes());
    buf.extend_from_slice(&p.fault_payload);
    buf
}

pub fn decode(buf: &[u8]) -> Result<StreamFaultPayload, CodecError> {
    if buf.len() < 4 {
        return Err(CodecError::TooShort { expected: 4, got: buf.len() });
    }
    let fault_code = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let fault_payload = buf[4..].to_vec();
    Ok(StreamFaultPayload { fault_code, fault_payload })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}
