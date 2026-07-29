//! CHANNEL_UNSUBSCRIBE / UNSUBSCRIBE_ACK codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsubscribePayload {
    pub subscription_id: uuid::Uuid,
    pub topic_hashes: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsubscribeAckPayload {
    pub subscription_id: uuid::Uuid,
}

pub fn decode(data: &[u8]) -> Result<UnsubscribePayload, CodecError> {
    if data.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, actual: data.len() });
    }
    let subscription_id = uuid::Uuid::from_bytes(data[0..16].try_into().unwrap());
    let topic_count = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;

    let mut offset = 24;
    let mut topic_hashes = Vec::with_capacity(topic_count);
    for _ in 0..topic_count {
        if offset + 32 > data.len() {
            return Err(CodecError::TruncatedTopics);
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&data[offset..offset + 32]);
        topic_hashes.push(hash);
        offset += 32;
    }

    Ok(UnsubscribePayload { subscription_id, topic_hashes })
}

pub fn encode(p: &UnsubscribePayload) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(p.subscription_id.as_bytes());
    let count = u32::try_from(p.topic_hashes.len()).expect("topic count exceeds u32");
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&[0u8; 4]); // reserved
    for hash in &p.topic_hashes {
        buf.extend_from_slice(hash);
    }
    buf
}

pub fn encode_ack(p: &UnsubscribeAckPayload) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(p.subscription_id.as_bytes());
    buf
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
    TruncatedTopics,
}
