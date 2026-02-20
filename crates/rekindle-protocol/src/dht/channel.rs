use crate::dht::DHTManager;
use crate::error::ProtocolError;
use serde::{Deserialize, Serialize};

/// A batch of messages stored in a single DHT record.
///
/// Channel messages form a linked list: each batch points to the
/// previous batch via `prev_record_key`. The latest batch key
/// is stored in the community record's channel entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageBatch {
    /// Link to the previous (older) message batch record.
    pub prev_record_key: Option<String>,
    /// Messages in this batch (newest last).
    pub messages: Vec<ChannelMessage>,
}

/// A single message within a channel message batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    /// Sender's Ed25519 public key (hex).
    pub sender_key: String,
    /// Message body (plaintext, after MEK decryption).
    pub body: String,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Nonce for deduplication.
    pub nonce: String,
    /// Reply-to nonce, if this is a reply.
    pub reply_to: Option<String>,
}

/// Maximum messages per batch (stay within DHT value size limits).
pub const MAX_MESSAGES_PER_BATCH: usize = 50;

/// Create a new message batch record and return its DHT key.
pub async fn create_batch(
    dht: &DHTManager,
    prev_key: Option<String>,
) -> Result<String, ProtocolError> {
    let batch = MessageBatch {
        prev_record_key: prev_key,
        messages: vec![],
    };

    let (key, _owner_keypair) = dht.create_record(1).await?;
    let data =
        serde_json::to_vec(&batch).map_err(|e| ProtocolError::Serialization(e.to_string()))?;
    dht.set_value(&key, 0, data).await?;

    Ok(key)
}

/// Append a message to the latest batch. If the batch is full,
/// creates a new batch and returns its key.
pub async fn append_message(
    dht: &DHTManager,
    batch_key: &str,
    message: ChannelMessage,
) -> Result<Option<String>, ProtocolError> {
    let mut batch = read_batch(dht, batch_key).await?;

    if batch.messages.len() >= MAX_MESSAGES_PER_BATCH {
        // Batch is full — create a new one linked to this one
        let new_key = create_batch(dht, Some(batch_key.to_string())).await?;
        let new_batch = MessageBatch {
            prev_record_key: Some(batch_key.to_string()),
            messages: vec![message],
        };
        let data = serde_json::to_vec(&new_batch)
            .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        dht.set_value(&new_key, 0, data).await?;
        return Ok(Some(new_key));
    }

    batch.messages.push(message);
    let data =
        serde_json::to_vec(&batch).map_err(|e| ProtocolError::Serialization(e.to_string()))?;
    dht.set_value(batch_key, 0, data).await?;

    Ok(None) // No new batch created
}

/// Read a message batch from DHT.
pub async fn read_batch(dht: &DHTManager, key: &str) -> Result<MessageBatch, ProtocolError> {
    match dht.get_value(key, 0).await? {
        Some(data) => {
            serde_json::from_slice(&data).map_err(|e| ProtocolError::Deserialization(e.to_string()))
        }
        None => Ok(MessageBatch {
            prev_record_key: None,
            messages: vec![],
        }),
    }
}

/// Read message history by following the linked list backwards.
///
/// Returns messages from newest to oldest, up to `limit` messages.
pub async fn read_history(
    dht: &DHTManager,
    latest_batch_key: &str,
    limit: usize,
) -> Result<Vec<ChannelMessage>, ProtocolError> {
    let mut all_messages = Vec::new();
    let mut current_key = Some(latest_batch_key.to_string());

    while let Some(key) = current_key {
        if all_messages.len() >= limit {
            break;
        }

        let batch = read_batch(dht, &key).await?;
        // Messages within a batch are oldest-first, but we want newest-first
        for msg in batch.messages.into_iter().rev() {
            all_messages.push(msg);
            if all_messages.len() >= limit {
                break;
            }
        }
        current_key = batch.prev_record_key;
    }

    Ok(all_messages)
}
