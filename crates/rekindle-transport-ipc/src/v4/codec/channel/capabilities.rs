//! CHANNEL_CAPABILITIES_QUERY / CAPABILITIES_REPLY codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitiesPayload {
    pub capabilities: u64,
}

pub fn decode(data: &[u8]) -> Result<CapabilitiesPayload, CodecError> {
    if data.len() < 8 {
        return Err(CodecError::TooShort { expected: 8, actual: data.len() });
    }
    Ok(CapabilitiesPayload {
        capabilities: u64::from_le_bytes(data[0..8].try_into().unwrap()),
    })
}

pub fn encode(p: &CapabilitiesPayload) -> Vec<u8> {
    p.capabilities.to_le_bytes().to_vec()
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
}
