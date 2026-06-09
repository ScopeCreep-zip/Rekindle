use crate::v3::wire::clearance::Clearance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatagramRequestPayload {
    pub message_id: uuid::Uuid,
    pub reply_timeout_ms: u32,
    pub sender_clearance: Clearance,
    pub application_payload: Vec<u8>,
    pub conditions: Vec<u8>,
}

pub fn encode(p: &DatagramRequestPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + p.application_payload.len() + p.conditions.len());
    buf.extend_from_slice(p.message_id.as_bytes());                          // 0..16
    buf.extend_from_slice(&p.reply_timeout_ms.to_le_bytes());                // 16..20
    buf.push(p.sender_clearance as u8);                                      // 20
    buf.extend_from_slice(&[0u8; 3]);                                        // 21..24 reserved
    crate::v3::codec::write_u32_len(&mut buf, p.application_payload.len());
    crate::v3::codec::write_u32_len(&mut buf, p.conditions.len());
    buf.extend_from_slice(&p.application_payload);
    buf.extend_from_slice(&p.conditions);
    buf
}

pub fn decode(buf: &[u8]) -> Result<DatagramRequestPayload, CodecError> {
    if buf.len() < 32 {
        return Err(CodecError::TooShort { expected: 32, got: buf.len() });
    }
    let message_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let reply_timeout_ms = u32::from_le_bytes(buf[16..20].try_into().unwrap());
    let sender_clearance = Clearance::try_from(buf[20]).map_err(|e| CodecError::InvalidClearance(e.0))?;
    let payload_len = u32::from_le_bytes(buf[24..28].try_into().unwrap()) as usize;
    let cond_len = u32::from_le_bytes(buf[28..32].try_into().unwrap()) as usize;
    if buf.len() < 32 + payload_len + cond_len {
        return Err(CodecError::TooShort { expected: 32 + payload_len + cond_len, got: buf.len() });
    }
    let application_payload = buf[32..32 + payload_len].to_vec();
    let conditions = buf[32 + payload_len..32 + payload_len + cond_len].to_vec();

    Ok(DatagramRequestPayload { message_id, reply_timeout_ms, sender_clearance, application_payload, conditions })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidClearance(u8),
}
