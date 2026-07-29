//! CHANNEL_ACK codec — batched MessageId acknowledgement.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckPayload {
    pub message_ids: Vec<uuid::Uuid>,
    pub batch_window_start_ns: u64,
}

pub fn decode(data: &[u8]) -> Result<AckPayload, CodecError> {
    if data.len() < 16 {
        return Err(CodecError::TooShort { expected: 16, actual: data.len() });
    }
    let count = u16::from_le_bytes([data[0], data[1]]) as usize;
    let batch_window_start_ns = u64::from_le_bytes(data[8..16].try_into().unwrap());

    let ids_start = 16;
    if data.len() < ids_start + count * 16 {
        return Err(CodecError::TruncatedIds);
    }

    let mut message_ids = Vec::with_capacity(count);
    for i in 0..count {
        let offset = ids_start + i * 16;
        let id = uuid::Uuid::from_bytes(data[offset..offset + 16].try_into().unwrap());
        message_ids.push(id);
    }

    Ok(AckPayload { message_ids, batch_window_start_ns })
}

pub fn encode(p: &AckPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16 + p.message_ids.len() * 16);
    let count = u16::try_from(p.message_ids.len()).expect("ack batch count exceeds u16");
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&[0u8; 6]); // reserved
    buf.extend_from_slice(&p.batch_window_start_ns.to_le_bytes());
    for id in &p.message_ids {
        buf.extend_from_slice(id.as_bytes());
    }
    buf
}

/// Convenience: encode a single-MessageId ACK.
pub fn encode_single(message_id: uuid::Uuid) -> Vec<u8> {
    encode(&AckPayload {
        message_ids: vec![message_id],
        batch_window_start_ns: 0,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
    TruncatedIds,
}
