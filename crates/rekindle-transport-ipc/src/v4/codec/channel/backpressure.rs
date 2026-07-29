//! CHANNEL_BACKPRESSURE / BACKPRESSURE_CLEAR codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackpressurePayload {
    pub resource_kind: u8,
    pub severity: u8,
    pub retry_after_ms: u32,
    pub resource_metric: u64,
}

pub fn decode(data: &[u8]) -> Result<BackpressurePayload, CodecError> {
    if data.len() < 16 {
        return Err(CodecError::TooShort { expected: 16, actual: data.len() });
    }
    Ok(BackpressurePayload {
        resource_kind: data[0],
        severity: data[1],
        retry_after_ms: u32::from_le_bytes(data[4..8].try_into().unwrap()),
        resource_metric: u64::from_le_bytes(data[8..16].try_into().unwrap()),
    })
}

pub fn encode(p: &BackpressurePayload) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0] = p.resource_kind;
    buf[1] = p.severity;
    buf[4..8].copy_from_slice(&p.retry_after_ms.to_le_bytes());
    buf[8..16].copy_from_slice(&p.resource_metric.to_le_bytes());
    buf
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
}
