//! CHANNEL_SUBSCRIBE / SUBSCRIBE_ACK codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribePayload {
    pub subscription_id: uuid::Uuid,
    pub topics: Vec<TopicEntry>,
    pub conditions: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicEntry {
    pub topic_hash: [u8; 32],
    pub topic_string: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeAckPayload {
    pub subscription_id: uuid::Uuid,
    pub accepted_hashes: Vec<[u8; 32]>,
}

pub fn decode(data: &[u8]) -> Result<SubscribePayload, CodecError> {
    if data.len() < 24 {
        return Err(CodecError::TooShort { expected: 24, actual: data.len() });
    }
    let subscription_id = uuid::Uuid::from_bytes(data[0..16].try_into().unwrap());
    let topic_count = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;
    let conditions_len = u32::from_le_bytes(data[20..24].try_into().unwrap()) as usize;

    let mut offset = 24;
    let mut topics = Vec::with_capacity(topic_count);
    for _ in 0..topic_count {
        if offset + 32 > data.len() {
            return Err(CodecError::TruncatedTopicTable);
        }
        let mut topic_hash = [0u8; 32];
        topic_hash.copy_from_slice(&data[offset..offset + 32]);
        offset += 32;

        if offset + 2 > data.len() {
            return Err(CodecError::TruncatedTopicTable);
        }
        let str_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        if offset + str_len > data.len() {
            return Err(CodecError::TruncatedTopicTable);
        }
        let topic_string = data[offset..offset + str_len].to_vec();
        offset += str_len;
        topics.push(TopicEntry { topic_hash, topic_string });
    }

    let conditions = if conditions_len > 0 && offset + conditions_len <= data.len() {
        data[offset..offset + conditions_len].to_vec()
    } else {
        vec![]
    };

    Ok(SubscribePayload { subscription_id, topics, conditions })
}

pub fn encode(p: &SubscribePayload) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(p.subscription_id.as_bytes());
    let topic_count = u32::try_from(p.topics.len()).expect("topic count exceeds u32");
    let cond_len = u32::try_from(p.conditions.len()).expect("conditions length exceeds u32");
    buf.extend_from_slice(&topic_count.to_le_bytes());
    buf.extend_from_slice(&cond_len.to_le_bytes());
    for topic in &p.topics {
        buf.extend_from_slice(&topic.topic_hash);
        let str_len = u16::try_from(topic.topic_string.len()).expect("topic string exceeds u16");
        buf.extend_from_slice(&str_len.to_le_bytes());
        buf.extend_from_slice(&topic.topic_string);
    }
    buf.extend_from_slice(&p.conditions);
    buf
}

pub fn encode_ack(p: &SubscribeAckPayload) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(p.subscription_id.as_bytes());
    let count = u32::try_from(p.accepted_hashes.len()).expect("accepted count exceeds u32");
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&[0u8; 4]); // reserved
    for hash in &p.accepted_hashes {
        buf.extend_from_slice(hash);
    }
    buf
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
    TruncatedTopicTable,
}
