//! Message envelope, payload enum, invite blobs, and RPC DTOs.
//!
//! Split by concern; every previously-public item is re-exported here
//! so `messaging::envelope::*` paths are unchanged.

mod dto;
mod invite;
mod payload;
mod sealing;

pub use dto::{
    AuditLogEntryDto, BannedMemberDto, CategoryDto, ChannelMessageDto, EventDto, EventRsvpDto,
    GameServerDto, InviteDto, MemberInfoDto, PinnedMessageDto, ReactionGroupDto, RoleDto,
    UnreadCountDto,
};
pub use invite::{
    check_invite_recency, create_invite_blob, decode_invite_url, encode_invite_url,
    verify_invite_blob, InviteBlob,
};
pub use payload::{GameInfo, MessagePayload};
pub use sealing::Sealing;

use serde::{Deserialize, Serialize};

/// This envelope provides sender identification and integrity verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Sender's Ed25519 public key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Message identity: kept across retries, so the receiver drops a
    /// second copy of a message it already accepted.
    pub nonce: Vec<u8>,
    /// The serialized `MessagePayload`: Signal ciphertext when its
    /// [`Sealing`] is `Session`, the plain JSON otherwise.
    pub payload: Vec<u8>,
    /// Ed25519 signature over
    /// [`crate::messaging::signing::envelope_signing_bytes`], which binds
    /// the recipient's identity key.
    pub signature: Vec<u8>,
}
