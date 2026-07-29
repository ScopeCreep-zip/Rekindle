use crate::v4::wire::clearance::Clearance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatagramNotifyPayload {
    pub message_id: uuid::Uuid,
    pub sender_clearance: Clearance,
    pub application_payload: Vec<u8>,
}

pub fn encode(p: &DatagramNotifyPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24 + p.application_payload.len());
    buf.extend_from_slice(p.message_id.as_bytes());                          // 0..16
    buf.push(p.sender_clearance as u8);                                      // 16
    buf.extend_from_slice(&[0u8; 3]);                                        // 17..20 reserved
    crate::v4::codec::write_u32_len(&mut buf, p.application_payload.len());
    buf.extend_from_slice(&p.application_payload);
    buf
}

pub fn decode(buf: &[u8]) -> Result<DatagramNotifyPayload, CodecError> {
    if buf.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, got: buf.len() });
    }
    let message_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let sender_clearance = Clearance::try_from(buf[16]).map_err(|e| CodecError::InvalidClearance(e.0))?;
    let payload_len = u32::from_le_bytes(buf[20..24].try_into().unwrap()) as usize;
    if buf.len() < 24 + payload_len {
        return Err(CodecError::TooShort { expected: 24 + payload_len, got: buf.len() });
    }
    let application_payload = buf[24..24 + payload_len].to_vec();

    Ok(DatagramNotifyPayload { message_id, sender_clearance, application_payload })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidClearance(u8),
}
