//! Wire types stored in channel record pages, plus the serde helpers
//! their field attributes reference.

use serde::{Deserialize, Serialize};

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
