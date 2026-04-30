//! DM (direct message) payload types.
//!
//! These are the inner payloads carried inside a [`SignedPayload`] envelope.
//! For session-based types (DirectMessage, Typing, etc.), the `body` is
//! Signal Protocol encrypted ciphertext. For session-establishing types
//! (FriendRequest, FriendAccept), the fields are plaintext.

use serde::{Deserialize, Serialize};

use crate::error::{TransportError, Result};
use crate::frame::TypeId;

/// All DM-class payload variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmPayload {
    /// Encrypted 1:1 chat message.
    DirectMessage {
        body: Vec<u8>,
        reply_to: Option<Vec<u8>>,
    },
    /// Typing indicator (ephemeral).
    Typing { typing: bool },
    /// Friend request (plaintext, TOFU signed).
    FriendRequest {
        display_name: String,
        message: String,
        prekey_bundle: Vec<u8>,
        profile_dht_key: String,
        route_blob: Vec<u8>,
        mailbox_dht_key: String,
        invite_id: Option<String>,
    },
    /// Friend request accepted.
    FriendAccept {
        prekey_bundle: Vec<u8>,
        profile_dht_key: String,
        route_blob: Vec<u8>,
        mailbox_dht_key: String,
        ephemeral_key: Vec<u8>,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
    },
    /// Friend request rejected.
    FriendReject,
    /// Delivery confirmation for a FriendRequest.
    FriendRequestAck,
    /// Notification that we have removed the peer as a friend.
    Unfriend,
    /// Acknowledgement of an Unfriend notification.
    UnfriendAck,
    /// Profile DHT key rotated (after block/unfriend).
    ProfileKeyRotated { new_profile_dht_key: String },
    /// Presence update (status, game info).
    PresenceUpdate {
        status: u8,
        game_info: Option<GamePresence>,
    },
}

/// Game presence information for DM presence updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePresence {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    pub server_address: Option<String>,
}

/// Deserialize a DM payload from raw bytes based on the frame TypeId.
pub fn deserialize_dm(type_id: TypeId, bytes: &[u8]) -> Result<DmPayload> {
    postcard::from_bytes(bytes).map_err(|e| TransportError::DeserializationFailed {
        type_id: type_id as u8,
        reason: e.to_string(),
    })
}

/// Serialize a DM payload to bytes.
pub fn serialize_dm(payload: &DmPayload) -> Result<Vec<u8>> {
    postcard::to_stdvec(payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

/// Map a DmPayload variant to its frame TypeId.
pub fn dm_type_id(payload: &DmPayload) -> TypeId {
    match payload {
        DmPayload::DirectMessage { .. } => TypeId::DmMessage,
        DmPayload::Typing { .. } => TypeId::DmTyping,
        DmPayload::FriendRequest { .. } => TypeId::FriendRequest,
        DmPayload::FriendAccept { .. } => TypeId::FriendAccept,
        DmPayload::FriendReject => TypeId::FriendReject,
        DmPayload::FriendRequestAck => TypeId::FriendRequestAck,
        DmPayload::Unfriend => TypeId::Unfriend,
        DmPayload::UnfriendAck => TypeId::UnfriendAck,
        DmPayload::ProfileKeyRotated { .. } => TypeId::ProfileKeyRotated,
        DmPayload::PresenceUpdate { .. } => TypeId::DmPresenceUpdate,
    }
}
