#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatagramReplyPayload {
    pub message_id: uuid::Uuid,
    pub correlation_id: uuid::Uuid,
    pub status_phase: u32,
    pub application_payload: Vec<u8>,
    pub status_reason: String,
    pub status_message: String,
    pub conditions: Vec<u8>,
}

pub fn encode(p: &DatagramReplyPayload) -> Vec<u8> {
    let reason_bytes = p.status_reason.as_bytes();
    let message_bytes = p.status_message.as_bytes();
    let mut buf = Vec::with_capacity(
        48 + p.application_payload.len() + reason_bytes.len() + message_bytes.len() + p.conditions.len(),
    );
    buf.extend_from_slice(p.message_id.as_bytes());                            // 0..16
    buf.extend_from_slice(p.correlation_id.as_bytes());                        // 16..32
    buf.extend_from_slice(&p.status_phase.to_le_bytes());                      // 32..36
    crate::v4::codec::write_u32_len(&mut buf, p.application_payload.len());
    crate::v4::codec::write_u16_len(&mut buf, reason_bytes.len());
    crate::v4::codec::write_u16_len(&mut buf, message_bytes.len());
    crate::v4::codec::write_u32_len(&mut buf, p.conditions.len());
    buf.extend_from_slice(&p.application_payload);
    buf.extend_from_slice(reason_bytes);
    buf.extend_from_slice(message_bytes);
    buf.extend_from_slice(&p.conditions);
    buf
}

pub fn decode(buf: &[u8]) -> Result<DatagramReplyPayload, CodecError> {
    if buf.len() < 48 {
        return Err(CodecError::TooShort { expected: 48, got: buf.len() });
    }
    let message_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let correlation_id = uuid::Uuid::from_bytes(buf[16..32].try_into().unwrap());
    let status_phase = u32::from_le_bytes(buf[32..36].try_into().unwrap());
    let payload_len = u32::from_le_bytes(buf[36..40].try_into().unwrap()) as usize;
    let reason_len = u16::from_le_bytes(buf[40..42].try_into().unwrap()) as usize;
    let message_len = u16::from_le_bytes(buf[42..44].try_into().unwrap()) as usize;
    let cond_len = u32::from_le_bytes(buf[44..48].try_into().unwrap()) as usize;

    let total = 48 + payload_len + reason_len + message_len + cond_len;
    if buf.len() < total {
        return Err(CodecError::TooShort { expected: total, got: buf.len() });
    }

    let mut offset = 48;
    let application_payload = buf[offset..offset + payload_len].to_vec();
    offset += payload_len;
    let status_reason = String::from_utf8(buf[offset..offset + reason_len].to_vec())
        .map_err(|_| CodecError::InvalidUtf8)?;
    offset += reason_len;
    let status_message = String::from_utf8(buf[offset..offset + message_len].to_vec())
        .map_err(|_| CodecError::InvalidUtf8)?;
    offset += message_len;
    let conditions = buf[offset..offset + cond_len].to_vec();

    Ok(DatagramReplyPayload {
        message_id, correlation_id, status_phase, application_payload,
        status_reason, status_message, conditions,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidUtf8,
}
