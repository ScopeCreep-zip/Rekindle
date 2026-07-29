//! CHANNEL_SIDECHANNEL_CREDIT codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidechannelCreditPayload {
    pub credit_fds: u16,
    pub credit_generation: u32,
    pub sidechannel_consumed: u64,
}

pub fn decode(data: &[u8]) -> Result<SidechannelCreditPayload, CodecError> {
    if data.len() < 16 {
        return Err(CodecError::TooShort { expected: 16, actual: data.len() });
    }
    Ok(SidechannelCreditPayload {
        credit_fds: u16::from_le_bytes([data[0], data[1]]),
        credit_generation: u32::from_le_bytes(data[4..8].try_into().unwrap()),
        sidechannel_consumed: u64::from_le_bytes(data[8..16].try_into().unwrap()),
    })
}

pub fn encode(p: &SidechannelCreditPayload) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0..2].copy_from_slice(&p.credit_fds.to_le_bytes());
    buf[4..8].copy_from_slice(&p.credit_generation.to_le_bytes());
    buf[8..16].copy_from_slice(&p.sidechannel_consumed.to_le_bytes());
    buf
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
}
