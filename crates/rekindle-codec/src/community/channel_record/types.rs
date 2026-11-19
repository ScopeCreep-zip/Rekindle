//! Wire types stored in channel record pages, plus the serde helpers
//! their field attributes reference.

use rekindle_types::base64_bytes;
use rekindle_types::domains;
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
    /// The message id of the message this one replies to, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// A thread reply's revival of a manually archived thread: the hex op
    /// hash of the `ThreadArchived` its author saw standing when it replied.
    /// It revives exactly that archive, never a later one (plan E3.3;
    /// architecture §4.6 "any new message ... implicitly unarchives it",
    /// ordered causally since no cross-author clock exists, rule E1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revives: Option<String>,
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
    /// The creator's nonce: the poll's id is derived from the creator's
    /// pseudonym and it (`rekindle_channel::poll_id_of`), so a create by
    /// any other member names another poll.
    pub nonce: [u8; 16],
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

/// A poll close entry on its author's channel outbox (plan E3.3). The
/// close names, for each voter its author counted, the stream seq of that
/// voter's vote, so readers count exactly those votes: a vote's validity
/// against the close is the closer's view of each voter's stream, never a
/// commit time (rule E1: no trusted clock; the departure-head pattern).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPollClose {
    /// Stable poll ID.
    pub poll_id: [u8; 16],
    /// Lamport logical timestamp for close ordering.
    pub lamport: u64,
    /// The votes the close counts, one per voter.
    pub counted: Vec<ChannelPollCounted>,
}

/// One voter's vote a poll close counts: the voter's pseudonym hex and the
/// stream seq of the vote counted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPollCounted {
    pub voter: String,
    pub stream_seq: u64,
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
    /// Source message id (string form, e.g. `"msg_<uuid>"`).
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

/// The author's edit of its own message `message_id` (plan E3.3, rule E1):
/// valid only from the message's author, and the latest by the author's
/// seq (`lamport_ts`, which only rises per author) wins. The body is sealed
/// at the edit entry's own position, not the message's.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelEdit {
    /// The edited message.
    pub message_id: String,
    /// The new body (sealed under the channel text key).
    #[serde(with = "base64_bytes")]
    pub ciphertext: Vec<u8>,
    /// The text key generation `ciphertext` is sealed under.
    pub mek_generation: u64,
    /// When the author edited it, in seconds.
    pub edited_at: u64,
    /// The author's seq: the edit's place among the author's entries.
    pub lamport_ts: u64,
}

/// The author's tombstone of its own message `message_id` (plan E3.3, rule
/// E3): allowed at any time and final, it purges every revision and later
/// edits are refused. The old ciphertext stays in the author's record and
/// in peers' copies, so it is "deleted for members", never erased (E4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDelete {
    /// The deleted message.
    pub message_id: String,
    /// When the author deleted it, in seconds.
    pub deleted_at: u64,
    /// The author's seq.
    pub lamport_ts: u64,
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
    Edit(ChannelEdit),
    Delete(ChannelDelete),
}

/// A decoded channel record entry together with the subkey it came from.
#[derive(Debug, Clone)]
pub struct ChannelRecordItem {
    pub subkey_index: u32,
    /// The author whose signature the slot value carries: an edit or a
    /// delete is applied only to that author's own message (plan E3.3 E1).
    pub author: rekindle_types::id::PseudonymKey,
    /// The seq of the slot value it was read from.
    pub seq: u32,
    /// The digest of that slot value's decrypted page
    /// (`rekindle_secrets::slot_head::slot_head_digest`): a departed
    /// member's post is kept only from the value its departure named
    /// (plan E3.2 S9).
    pub slot_digest: [u8; 32],
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
            Self::Edit(edit) => edit.lamport_ts,
            Self::Delete(delete) => delete.lamport_ts,
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
    /// 64-byte Ed25519 signature over [`ChannelSubkeyPayload::signing_bytes`]. Empty `Vec`
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
            Vec::with_capacity(domains::CHANNEL_SUBKEY.len() + 32 + 8 + entries_json.len());
        out.extend_from_slice(domains::CHANNEL_SUBKEY.as_bytes());
        out.extend_from_slice(&self.author_pseudonym.0);
        out.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        out.extend_from_slice(&entries_json);
        out
    }
}
