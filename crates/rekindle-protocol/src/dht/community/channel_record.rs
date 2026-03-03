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

/// Channel record header stored in subkey 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelHeader {
    /// Channel ID this record belongs to.
    pub channel_id: String,
    /// Community ID this channel is part of.
    pub community_id: String,
    /// Latest message sequence number (monotonically increasing).
    pub latest_sequence: u64,
    /// Current MEK generation for this channel.
    pub mek_generation: u64,
    /// Timestamp of the last message.
    pub last_message_at: u64,
}

/// A message entry written to a channel record subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessage {
    /// Message sequence number assigned by the coordinator.
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
}

/// Create a new channel record (DFLT for now, will be upgraded to SMPL when members join).
///
/// Returns `(record_key, owner_keypair)`.
pub async fn create_channel_record(
    dht: &DHTManager,
    channel_id: &str,
    community_id: &str,
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    let (key, owner_keypair) = dht
        .create_record(u32::from(CHANNEL_OWNER_SUBKEY_COUNT))
        .await?;

    // Write initial channel header
    let header = ChannelHeader {
        channel_id: channel_id.to_string(),
        community_id: community_id.to_string(),
        latest_sequence: 0,
        mek_generation: 0,
        last_message_at: 0,
    };
    write_header(dht, &key, &header).await?;

    tracing::debug!(key = %key, channel = %channel_id, "channel record created");
    Ok((key, owner_keypair))
}

/// Read the channel record header.
pub async fn read_header(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<ChannelHeader>, ProtocolError> {
    match dht.get_value(key, CHANNEL_HEADER_SUBKEY).await? {
        Some(data) => {
            let header: ChannelHeader = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("channel header: {e}")))?;
            Ok(Some(header))
        }
        None => Ok(None),
    }
}

/// Write the channel record header (coordinator only).
pub async fn write_header(
    dht: &DHTManager,
    key: &str,
    header: &ChannelHeader,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(header)
        .map_err(|e| ProtocolError::Serialization(format!("channel header: {e}")))?;
    dht.set_value(key, CHANNEL_HEADER_SUBKEY, bytes).await
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
