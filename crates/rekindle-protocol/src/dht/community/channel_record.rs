//! Per-channel SMPL message records.
//!
//! Each channel has its own DHT record for storing message history.
//! Channel records use zero-owner SMPL: each member writes directly to the
//! subkey matching their registry slot.

use serde::{Deserialize, Serialize};

use crate::dht::DHTManager;
use crate::error::ProtocolError;

/// No dedicated header subkey exists in v2.0 channel records.
pub const CHANNEL_HEADER_SUBKEY: u32 = 0;

/// Channel SMPL records use `o_cnt:0`; member slots start at subkey 0.
pub const CHANNEL_OWNER_SUBKEY_COUNT: u16 = 0;

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

/// A durable reaction entry written to a channel record subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelReaction {
    /// Target message ID.
    pub message_id: String,
    /// Unicode emoji or `custom:{expression_id_hex}`.
    pub expression: String,
    /// true = add, false = remove.
    pub added: bool,
    /// Lamport logical timestamp for LWW merge.
    pub lamport: u64,
}

/// A durable poll creation entry written to a channel record subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPollCreate {
    /// Stable poll ID.
    pub poll_id: [u8; 16],
    /// Message the poll is attached to.
    pub message_id: String,
    pub question: String,
    pub answers: Vec<String>,
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Lamport logical timestamp for author-bound LWW merge.
    pub lamport: u64,
}

/// A durable poll vote entry written to a channel record subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPollVote {
    /// Stable poll ID.
    pub poll_id: [u8; 16],
    /// Selected answer indices.
    pub selected_answers: Vec<u8>,
    /// Lamport logical timestamp for voter-local LWW merge.
    pub lamport: u64,
}

/// A durable poll close entry written to a channel record subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPollClose {
    /// Stable poll ID.
    pub poll_id: [u8; 16],
    /// Lamport logical timestamp for close ordering.
    pub lamport: u64,
}

/// Any durable entry stored in a channel record page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ChannelRecordEntry {
    Message(ChannelMessage),
    Reaction(ChannelReaction),
    PollCreate(ChannelPollCreate),
    PollVote(ChannelPollVote),
    PollClose(ChannelPollClose),
}

/// A decoded channel record entry together with the subkey it came from.
#[derive(Debug, Clone)]
pub struct ChannelRecordItem {
    pub subkey_index: u32,
    pub entry: ChannelRecordEntry,
}

impl ChannelRecordEntry {
    pub fn lamport(&self) -> u64 {
        match self {
            Self::Message(message) => message.lamport_ts,
            Self::Reaction(reaction) => reaction.lamport,
            Self::PollCreate(create) => create.lamport,
            Self::PollVote(vote) => vote.lamport,
            Self::PollClose(close) => close.lamport,
        }
    }
}

pub fn decode_channel_entries(data: &[u8]) -> Result<Vec<ChannelRecordEntry>, ProtocolError> {
    if let Ok(entries) = serde_json::from_slice::<Vec<ChannelRecordEntry>>(data) {
        return Ok(entries);
    }
    serde_json::from_slice::<Vec<ChannelMessage>>(data)
        .map(|messages| {
            messages
                .into_iter()
                .map(ChannelRecordEntry::Message)
                .collect()
        })
        .map_err(|e| ProtocolError::Deserialization(format!("channel entries: {e}")))
}

fn encode_page_entries(entries: &[ChannelRecordEntry]) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(entries)
        .map_err(|e| ProtocolError::Serialization(format!("channel entries: {e}")))
}

/// Read messages from a specific subkey in the channel record.
pub async fn read_messages(
    dht: &DHTManager,
    key: &str,
    subkey: u32,
) -> Result<Vec<ChannelMessage>, ProtocolError> {
    match dht.get_value(key, subkey).await? {
        Some(data) => decode_channel_entries(&data).map(|entries| {
            entries
                .into_iter()
                .filter_map(|entry| match entry {
                    ChannelRecordEntry::Message(message) => Some(message),
                    ChannelRecordEntry::Reaction(_)
                    | ChannelRecordEntry::PollCreate(_)
                    | ChannelRecordEntry::PollVote(_)
                    | ChannelRecordEntry::PollClose(_) => None,
                })
                .collect()
        }),
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
    let entries: Vec<ChannelRecordEntry> = messages
        .iter()
        .cloned()
        .map(ChannelRecordEntry::Message)
        .collect();
    let bytes = encode_page_entries(&entries)?;
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
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        ChannelRecordEntry::Message(message.clone()),
    )
    .await
}

/// Write a reaction entry to a member's subkey in the channel SMPL record.
pub async fn write_member_reaction(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    reaction: &ChannelReaction,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        ChannelRecordEntry::Reaction(reaction.clone()),
    )
    .await
}

/// Write a poll create entry to a member's subkey in the channel SMPL record.
pub async fn write_member_poll_create(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    poll_create: &ChannelPollCreate,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        ChannelRecordEntry::PollCreate(poll_create.clone()),
    )
    .await
}

/// Write a poll vote entry to a member's subkey in the channel SMPL record.
pub async fn write_member_poll_vote(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    poll_vote: &ChannelPollVote,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        ChannelRecordEntry::PollVote(poll_vote.clone()),
    )
    .await
}

/// Write a poll close entry to a member's subkey in the channel SMPL record.
pub async fn write_member_poll_close(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    poll_close: &ChannelPollClose,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        ChannelRecordEntry::PollClose(poll_close.clone()),
    )
    .await
}

fn message_from_entry(entry: &ChannelRecordEntry) -> Option<&ChannelMessage> {
    match entry {
        ChannelRecordEntry::Message(message) => Some(message),
        ChannelRecordEntry::Reaction(_)
        | ChannelRecordEntry::PollCreate(_)
        | ChannelRecordEntry::PollVote(_)
        | ChannelRecordEntry::PollClose(_) => None,
    }
}

async fn write_member_entry(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    entry: ChannelRecordEntry,
) -> Result<(), ProtocolError> {
    let subkey = u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + member_index;

    let mut entries = match dht.get_value(channel_key, subkey).await? {
        Some(data) => decode_channel_entries(&data).unwrap_or_default(),
        None => Vec::new(),
    };

    entries.push(entry);

    let mut bytes = encode_page_entries(&entries)?;
    while bytes.len() > MAX_PAGE_SIZE && entries.len() > 1 {
        entries.remove(0);
        bytes = encode_page_entries(&entries)?;
    }

    dht.set_value_with_writer(channel_key, subkey, bytes, writer)
        .await
}

/// Decode all durable entries from all member subkeys in the channel SMPL record.
pub async fn read_all_channel_entries(
    rc: &veilid_core::RoutingContext,
    channel_key: &str,
    member_count: u32,
) -> Result<Vec<ChannelRecordItem>, ProtocolError> {
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
            (subkey, result)
        });
    }

    let mut items = Vec::new();
    while let Some((subkey_index, result)) = futs.next().await {
        if let Ok(Some(data)) = result {
            if let Ok(entries) = decode_channel_entries(&data) {
                items.extend(entries.into_iter().map(|entry| ChannelRecordItem {
                    subkey_index,
                    entry,
                }));
            }
        }
    }

    items.sort_by(|a, b| {
        a.entry
            .lamport()
            .cmp(&b.entry.lamport())
            .then_with(|| a.subkey_index.cmp(&b.subkey_index))
    });
    Ok(items)
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
    let mut all_messages: Vec<ChannelMessage> =
        read_all_channel_entries(rc, channel_key, member_count)
            .await?
            .into_iter()
            .filter_map(|item| message_from_entry(&item.entry).cloned())
            .collect();

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

#[cfg(test)]
mod tests;
