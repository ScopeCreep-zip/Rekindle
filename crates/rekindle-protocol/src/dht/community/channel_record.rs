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
    /// Lost Cargo: optional file attachment offer (architecture §28.9
    /// line 3233 — offer travels embedded in the chat message). Serialized
    /// as a missing key for plain messages so legacy peers parse correctly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<rekindle_types::attachment::AttachmentOffer>,
    /// Per `ChannelEntry::Message.flags` bitfield — VOICE_MESSAGE=0x10,
    /// SUPPRESS_NOTIFICATIONS=0x20, etc. See `rekindle_types::channel::flags`.
    /// Defaults to 0 for plain messages so legacy payloads parse unchanged.
    #[serde(default)]
    pub flags: u32,
    /// Architecture §28.5 line 3105-3120 — cleartext mention metadata.
    /// Receivers route notifications **before** decrypting the body
    /// (mentions are visible to all gossip participants anyway, since
    /// the message body — once decrypted — would name the same
    /// pseudonyms in plaintext per spec line 3116). The
    /// `@everyone` / `@here` cleartext signals live in `flags`
    /// (`MENTION_EVERYONE = 0x40`, `MENTION_HERE = 0x80`) so
    /// non-mention messages stay byte-for-byte identical to the legacy
    /// wire shape. Reader-validates: peers reject those bits from
    /// senders without `MENTION_EVERYONE` permission (§9.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentioned_pseudonyms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentioned_roles: Vec<String>,
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

/// A durable hand-raise entry written to a channel record subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelHandRaise {
    /// true = raise hand, false = lower hand.
    pub raised: bool,
    /// Lamport logical timestamp for LWW merge.
    pub lamport: u64,
}

/// Lost Cargo: a peer's advertisement of which chunks of an attachment
/// it has cached locally (architecture §28.9 lines 3268-3274 + plan
/// §1.J4 bitmap). Downloaders scan all member subkeys for these entries
/// to find sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAttachmentCached {
    pub attachment_id: [u8; 16],
    /// LSB-first bit per chunk; length = `ceil(chunk_count / 8)`.
    #[serde(with = "base64_bytes")]
    pub chunk_bitmap: Vec<u8>,
    /// Total chunks of the file. Receivers reject entries whose bitmap
    /// length does not match `ceil(chunk_count / 8)`.
    pub chunk_count: u32,
    pub author_pseudonym: String,
    /// Lamport logical timestamp for LWW per (author_pseudonym, attachment_id).
    pub lamport_ts: u64,
}

/// A durable forwarded-message entry written to a channel record subkey.
///
/// Forwarding re-encrypts the source message body with the destination
/// channel's MEK so destination members can decrypt without needing the
/// source community's key. The `original_author` pseudonym is preserved
/// for display attribution; cross-community pseudonyms use independent
/// derivations so the value is NOT linkable across communities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelForward {
    /// Sequence number assigned by the forwarder for this dest-channel write.
    pub sequence: u64,
    /// Forwarder's pseudonym public key (hex) in the destination community.
    pub sender_pseudonym: String,
    /// Source message id (string form, e.g. "msg_<uuid>").
    pub original_message_id: String,
    /// Source channel id (hex, 16 bytes).
    pub original_channel_id: String,
    /// Source author's pseudonym (hex, 32 bytes) for display only.
    pub original_author: String,
    /// Source body re-encrypted with the destination channel's MEK.
    #[serde(with = "base64_bytes")]
    pub content_snapshot: Vec<u8>,
    /// MEK generation used for `content_snapshot`.
    pub mek_generation: u64,
    /// Unix timestamp (milliseconds) of the forward write.
    pub timestamp: u64,
    /// Lamport logical timestamp for causal ordering at the destination.
    #[serde(default)]
    pub lamport_ts: u64,
    /// Unique forward id (for deduplication on the destination channel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Any durable entry stored in a channel record page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ChannelRecordEntry {
    Message(ChannelMessage),
    Forward(ChannelForward),
    AttachmentCached(ChannelAttachmentCached),
    Reaction(ChannelReaction),
    PollCreate(ChannelPollCreate),
    PollVote(ChannelPollVote),
    PollClose(ChannelPollClose),
    HandRaise(ChannelHandRaise),
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
            Self::Forward(forward) => forward.lamport_ts,
            Self::AttachmentCached(cached) => cached.lamport_ts,
            Self::Reaction(reaction) => reaction.lamport,
            Self::PollCreate(create) => create.lamport,
            Self::PollVote(vote) => vote.lamport,
            Self::PollClose(close) => close.lamport,
            Self::HandRaise(hand_raise) => hand_raise.lamport,
        }
    }
}

/// Architecture §26 W26 — wraps the per-subkey entry vec with the
/// author's pseudonym + an Ed25519 signature. The SMPL slot keypair on
/// `set_dht_value` is community-shared (every member knows the slot
/// seed), so without this wrapper any member could forge entries
/// claiming to be any other member. Receivers MUST verify the signature
/// against `author_pseudonym` before treating the entries as authentic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSubkeyPayload {
    pub author_pseudonym: rekindle_types::id::PseudonymKey,
    pub entries: Vec<ChannelRecordEntry>,
    /// 64-byte Ed25519 signature over [`signing_bytes`]. Empty `Vec`
    /// only on legacy/disk fixtures predating SCHEMA_VERSION 59.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<u8>,
}

impl ChannelSubkeyPayload {
    /// Canonical bytes the author signs. Includes a domain tag so the
    /// signature can't be lifted into a different protocol context
    /// (governance, presence, gossip). Includes the entry count so a
    /// truncated re-write by a forger is rejected.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let entries_json = serde_json::to_vec(&self.entries).unwrap_or_default();
        let mut out =
            Vec::with_capacity(b"rekindle-channel-subkey-v1".len() + 32 + 8 + entries_json.len());
        out.extend_from_slice(b"rekindle-channel-subkey-v1");
        out.extend_from_slice(&self.author_pseudonym.0);
        out.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        out.extend_from_slice(&entries_json);
        out
    }
}

/// Decode raw subkey bytes into the entry vec, verifying the author's
/// pseudonym signature AND that every entry's per-record sender field
/// matches the wrapper-level author. Returns `Err` for any signature
/// failure, length mismatch, or per-entry sender forgery so callers
/// don't quietly treat forged entries as authentic.
///
/// Architecture §26 W26 — three entry variants carry their own sender
/// field (`ChannelMessage.sender_pseudonym`,
/// `ChannelForward.sender_pseudonym`,
/// `ChannelAttachmentCached.author_pseudonym`). Without this binding the
/// wrapper signature would only prove "signed by author X"; X could
/// then write entries claiming `sender_pseudonym = victim` and the
/// receiver would attribute them to victim. We reject the entire
/// payload on any mismatch — a single forged entry taints the rest
/// because they all flowed through the same writer who tried to lie.
pub fn decode_channel_entries(data: &[u8]) -> Result<Vec<ChannelRecordEntry>, ProtocolError> {
    let payload: ChannelSubkeyPayload = serde_json::from_slice(data)
        .map_err(|e| ProtocolError::Deserialization(format!("channel subkey: {e}")))?;
    let sig_arr: [u8; 64] = payload
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolError::Verification("channel subkey signature length".into()))?;
    rekindle_secrets::derive::verify_pseudonym_signature(
        &payload.author_pseudonym.0,
        &payload.signing_bytes(),
        &sig_arr,
    )
    .map_err(|e| ProtocolError::Verification(format!("channel subkey signature: {e}")))?;
    let author_hex = hex::encode(payload.author_pseudonym.0);
    for entry in &payload.entries {
        match entry {
            ChannelRecordEntry::Message(msg) => {
                if !msg.sender_pseudonym.eq_ignore_ascii_case(&author_hex) {
                    return Err(ProtocolError::Verification(
                        "channel message sender_pseudonym does not match subkey author".into(),
                    ));
                }
            }
            ChannelRecordEntry::Forward(fwd) => {
                if !fwd.sender_pseudonym.eq_ignore_ascii_case(&author_hex) {
                    return Err(ProtocolError::Verification(
                        "channel forward sender_pseudonym does not match subkey author".into(),
                    ));
                }
            }
            ChannelRecordEntry::AttachmentCached(att) => {
                if !att.author_pseudonym.eq_ignore_ascii_case(&author_hex) {
                    return Err(ProtocolError::Verification(
                        "channel attachment-cached author_pseudonym does not match subkey author"
                            .into(),
                    ));
                }
            }
            ChannelRecordEntry::Reaction(_)
            | ChannelRecordEntry::HandRaise(_)
            | ChannelRecordEntry::PollCreate(_)
            | ChannelRecordEntry::PollVote(_)
            | ChannelRecordEntry::PollClose(_) => {
                // No sender field; attribution is by subkey ownership
                // (the wrapper-level author), which is already verified.
            }
        }
    }
    Ok(payload.entries)
}

fn encode_page_entries(
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    entries: Vec<ChannelRecordEntry>,
) -> Result<Vec<u8>, ProtocolError> {
    let mut payload = ChannelSubkeyPayload {
        author_pseudonym,
        entries,
        signature: Vec::new(),
    };
    let sig =
        rekindle_secrets::derive::sign_with_pseudonym(pseudonym_signing_key, &payload.signing_bytes());
    payload.signature = sig.to_vec();
    serde_json::to_vec(&payload)
        .map_err(|e| ProtocolError::Serialization(format!("channel subkey: {e}")))
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
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    message: &ChannelMessage,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
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
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    reaction: &ChannelReaction,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
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
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_create: &ChannelPollCreate,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
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
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_vote: &ChannelPollVote,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
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
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_close: &ChannelPollClose,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::PollClose(poll_close.clone()),
    )
    .await
}

/// Write a hand-raise entry to a member's subkey in the channel SMPL record.
pub async fn write_member_hand_raise(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    hand_raise: &ChannelHandRaise,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::HandRaise(hand_raise.clone()),
    )
    .await
}

/// Write an `AttachmentCached` entry to a member's subkey in the channel SMPL
/// record (Lost Cargo source advertisement, architecture §28.9 line 3268).
pub async fn write_member_attachment_cached(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    cached: &ChannelAttachmentCached,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::AttachmentCached(cached.clone()),
    )
    .await
}

/// Write a forwarded-message entry to a member's subkey in the channel SMPL record.
///
/// Forwards are stored as their own variant (not wrapped in `Message`) so the
/// destination channel's UI can render the "Forwarded from" attribution
/// without re-parsing message bodies.
pub async fn write_member_forward(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    forward: &ChannelForward,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::Forward(forward.clone()),
    )
    .await
}

fn message_from_entry(entry: &ChannelRecordEntry) -> Option<&ChannelMessage> {
    match entry {
        ChannelRecordEntry::Message(message) => Some(message),
        ChannelRecordEntry::Forward(_)
        | ChannelRecordEntry::AttachmentCached(_)
        | ChannelRecordEntry::Reaction(_)
        | ChannelRecordEntry::PollCreate(_)
        | ChannelRecordEntry::PollVote(_)
        | ChannelRecordEntry::PollClose(_)
        | ChannelRecordEntry::HandRaise(_) => None,
    }
}

async fn write_member_entry(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    entry: ChannelRecordEntry,
) -> Result<(), ProtocolError> {
    let subkey = u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + member_index;

    // Architecture §26 W26 — only inherit existing entries that pass the
    // author signature check. Otherwise a forger who overwrote our
    // subkey via the shared slot_seed would launder their entries into
    // every subsequent legitimate write we make.
    let mut entries = match dht.get_value(channel_key, subkey).await? {
        Some(data) => decode_channel_entries(&data).unwrap_or_default(),
        None => Vec::new(),
    };

    entries.push(entry);

    let mut bytes = encode_page_entries(
        author_pseudonym.clone(),
        pseudonym_signing_key,
        entries.clone(),
    )?;
    while bytes.len() > MAX_PAGE_SIZE && entries.len() > 1 {
        entries.remove(0);
        bytes = encode_page_entries(
            author_pseudonym.clone(),
            pseudonym_signing_key,
            entries.clone(),
        )?;
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
