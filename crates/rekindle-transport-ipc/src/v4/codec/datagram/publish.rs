use crate::v4::wire::clearance::Clearance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatagramPublishPayload {
    pub message_id: uuid::Uuid,
    pub topic_hash: [u8; 32],
    pub event_seq: u32,
    pub event_timestamp_ns: u64,
    pub sender_clearance: Clearance,
    pub application_payload: Vec<u8>,
    pub conditions: Vec<u8>,
}

pub fn encode(p: &DatagramPublishPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(72 + p.application_payload.len() + p.conditions.len());
    buf.extend_from_slice(p.message_id.as_bytes());                          // 0..16
    buf.extend_from_slice(&p.topic_hash);                                    // 16..48
    buf.extend_from_slice(&p.event_seq.to_le_bytes());                       // 48..52
    crate::v4::codec::write_u32_len(&mut buf, p.application_payload.len());
    buf.extend_from_slice(&p.event_timestamp_ns.to_le_bytes());
    buf.push(p.sender_clearance as u8);
    buf.extend_from_slice(&[0u8; 3]);
    crate::v4::codec::write_u32_len(&mut buf, p.conditions.len());
    buf.extend_from_slice(&p.application_payload);
    buf.extend_from_slice(&p.conditions);
    buf
}

pub fn decode(buf: &[u8]) -> Result<DatagramPublishPayload, CodecError> {
    if buf.len() < 72 {
        return Err(CodecError::TooShort { expected: 72, got: buf.len() });
    }
    let message_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let mut topic_hash = [0u8; 32];
    topic_hash.copy_from_slice(&buf[16..48]);
    let event_seq = u32::from_le_bytes(buf[48..52].try_into().unwrap());
    let payload_len = u32::from_le_bytes(buf[52..56].try_into().unwrap()) as usize;
    let event_timestamp_ns = u64::from_le_bytes(buf[56..64].try_into().unwrap());
    let sender_clearance = Clearance::try_from(buf[64]).map_err(|e| CodecError::InvalidClearance(e.0))?;
    let cond_len = u32::from_le_bytes(buf[68..72].try_into().unwrap()) as usize;

    if buf.len() < 72 + payload_len + cond_len {
        return Err(CodecError::TooShort { expected: 72 + payload_len + cond_len, got: buf.len() });
    }
    let application_payload = buf[72..72 + payload_len].to_vec();
    let conditions = buf[72 + payload_len..72 + payload_len + cond_len].to_vec();

    Ok(DatagramPublishPayload {
        message_id, topic_hash, event_seq, event_timestamp_ns,
        sender_clearance, application_payload, conditions,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidClearance(u8),
}
