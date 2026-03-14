//! Per-channel SMPL message records.
//!
//! Each channel has its own DHT record for storing message history.
//! The coordinator owns subkeys 0 (channel metadata/latest sequence) and
//! additional subkeys for message storage. Members can append messages
//! to their assigned subkeys.

use serde::{Deserialize, Serialize};

use crate::dht::DHTManager;
use crate::error::ProtocolError;

/// Subkey 0: Channel record header (owned by coordinator).
pub const CHANNEL_HEADER_SUBKEY: u32 = 0;

/// Owner subkey count for the coordinator (header + message buffer).
pub const CHANNEL_OWNER_SUBKEY_COUNT: u16 = 8;

/// Each member gets 1 subkey for message submission.
pub const CHANNEL_MEMBER_SUBKEY_COUNT: u16 = 1;

/// A message entry written to a channel record subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessage {
    /// Message sequence number assigned by the sender.
    pub sequence: u64,
    /// Sender's pseudonym public key (hex).
    pub sender_pseudonym: String,
    /// Encrypted message body (MEK-encrypted).
    #[serde(with = "base64_bytes")]
    pub ciphertext: Vec<u8>,
    /// MEK generation used for encryption.
    pub mek_generation: u64,
    /// Unix timestamp (milliseconds).
    pub timestamp: u64,
    /// Optional reply-to sequence number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<u64>,
    /// Lamport logical timestamp for causal ordering.
    #[serde(default)]
    pub lamport_ts: u64,
    /// Unique message ID (for deduplication).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Read messages from a specific subkey in the channel record.
pub async fn read_messages(
    dht: &DHTManager,
    key: &str,
    subkey: u32,
) -> Result<Vec<ChannelMessage>, ProtocolError> {
    match dht.get_value(key, subkey).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("channel messages: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write messages to a specific subkey in the channel record.
pub async fn write_messages(
    dht: &DHTManager,
    key: &str,
    subkey: u32,
    messages: &[ChannelMessage],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(messages)
        .map_err(|e| ProtocolError::Serialization(format!("channel messages: {e}")))?;
    dht.set_value(key, subkey, bytes).await
}

/// Watch a channel record for new messages.
pub async fn watch_channel(
    dht: &DHTManager,
    key: &str,
    subkey_count: u32,
) -> Result<bool, ProtocolError> {
    let subkeys: Vec<u32> = (0..subkey_count).collect();
    dht.watch_record(key, &subkeys).await
}

// ── SMPL multi-writer channel persistence ──

/// Maximum serialized size for a member's message page (~30KB, leaving DHT overhead room).
const MAX_PAGE_SIZE: usize = 30_000;

/// Create a new SMPL channel record with pre-allocated member slots.
///
/// Uses the same slot seed as the member registry so members derive their
/// writer keypair independently via `derive_slot_veilid_keypair(seed, slot_index)`.
/// Returns `(record_key, owner_keypair)`.
pub async fn create_smpl_channel_record(
    dht: &DHTManager,
    slot_seed: &[u8; 32],
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    use crate::dht::community::member_registry;

    let mut members = Vec::with_capacity(member_registry::SLOTS_PER_SEGMENT as usize);
    for i in 0..member_registry::SLOTS_PER_SEGMENT {
        let signing_key = member_registry::derive_slot_keypair(slot_seed, i)?;
        let public_bytes = signing_key.verifying_key().to_bytes();
        members.push(veilid_core::DHTSchemaSMPLMember {
            m_key: veilid_core::BareMemberId::new(&public_bytes),
            m_cnt: CHANNEL_MEMBER_SUBKEY_COUNT,
        });
    }

    let (key, owner_keypair) = dht
        .create_smpl_record(CHANNEL_OWNER_SUBKEY_COUNT, members)
        .await?;

    tracing::debug!(key = %key, "SMPL channel record created");
    Ok((key, owner_keypair))
}

/// Write a message to a member's subkey in the channel SMPL record.
///
/// Reads existing messages from the subkey, appends the new one, and writes back.
/// If the page exceeds MAX_PAGE_SIZE, oldest messages are dropped from DHT
/// (they're still in SQLite locally).
pub async fn write_member_message(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    message: &ChannelMessage,
) -> Result<(), ProtocolError> {
    let subkey = u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + member_index;

    // Read existing messages from our subkey
    let mut messages = match dht.get_value(channel_key, subkey).await? {
        Some(data) => serde_json::from_slice::<Vec<ChannelMessage>>(&data).unwrap_or_default(),
        None => Vec::new(),
    };

    // Append new message
    messages.push(message.clone());

    // Trim oldest if over size limit
    let mut bytes = serde_json::to_vec(&messages)
        .map_err(|e| ProtocolError::Serialization(format!("channel messages: {e}")))?;
    while bytes.len() > MAX_PAGE_SIZE && messages.len() > 1 {
        messages.remove(0);
        bytes = serde_json::to_vec(&messages)
            .map_err(|e| ProtocolError::Serialization(format!("channel messages: {e}")))?;
    }

    // Write with member's keypair
    dht.set_value_with_writer(channel_key, subkey, bytes, writer).await
}

/// Read all messages from all member subkeys in the channel SMPL record.
///
/// Returns messages sorted by (lamport_ts, sender_pseudonym) for deterministic
/// ordering. Uses parallel reads bounded by a semaphore.
pub async fn read_all_channel_messages(
    rc: &veilid_core::RoutingContext,
    channel_key: &str,
    member_count: u32,
) -> Result<Vec<ChannelMessage>, ProtocolError> {
    use futures::stream::{FuturesUnordered, StreamExt};

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
    let mut futs = FuturesUnordered::new();

    for i in 0..member_count {
        let sem = sem.clone();
        let rc = rc.clone();
        let key = channel_key.to_string();
        let subkey = u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + i;
        futs.push(async move {
            let permit = sem.acquire().await.unwrap();
            let mgr = DHTManager::new(rc);
            let result = mgr.get_value(&key, subkey).await;
            drop(permit);
            result
        });
    }

    let mut all_messages = Vec::new();
    while let Some(result) = futs.next().await {
        if let Ok(Some(data)) = result {
            if let Ok(msgs) = serde_json::from_slice::<Vec<ChannelMessage>>(&data) {
                all_messages.extend(msgs);
            }
        }
    }

    // Sort by (lamport_ts, sender_pseudonym) for deterministic ordering
    all_messages.sort_by(|a, b| {
        a.lamport_ts
            .cmp(&b.lamport_ts)
            .then_with(|| a.sender_pseudonym.cmp(&b.sender_pseudonym))
    });

    Ok(all_messages)
}

/// Serde helper for base64-encoding Vec<u8> fields in JSON.
mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&b64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}
