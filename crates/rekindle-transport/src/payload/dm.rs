//! DM (direct message) payload types.
//!
//! These are the inner payloads carried inside a [`SignedPayload`] envelope.
//! For session-based types (DirectMessage, Typing, etc.), the `body` is
//! Signal Protocol encrypted ciphertext. For session-establishing types
//! (FriendRequest, FriendAccept), the fields are plaintext.

use serde::{Deserialize, Serialize};

use crate::error::{TransportError, Result};
use crate::frame::TypeId;

/// DM-class payload variants for ephemeral peer-to-peer signals.
///
/// DM message content is NOT carried here — it goes through Signal-encrypted
/// DhtLog entries (see broadcast/dm.rs). This enum carries only ephemeral
/// signals that don't need persistence or encryption (typing, presence,
/// friend lifecycle notifications).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmPayload {
    /// Typing indicator (ephemeral, real-time only).
    Typing { typing: bool },
    /// Delivery confirmation for a friend request written to DHT inbox.
    /// Triggers an immediate inbox scan on the recipient so they discover
    /// the request without waiting for the 60s poll sweep.
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

// ── SubscriptionEvent conversion ───────────────────────────────────────

use rekindle_types::subscription_events::{
    SubscriptionEvent,
    TypingEvent, TypingContext,
    FriendEvent, PresenceEvent,
};

impl DmPayload {
    /// Convert a DM payload into a `SubscriptionEvent` given sender context.
    ///
    /// Only ephemeral signals remain — DM message content goes through
    /// Signal-encrypted DhtLog entries, not this conversion.
    pub fn into_event(self, sender_key: &str) -> SubscriptionEvent {
        match self {
            Self::Typing { typing } => {
                if typing {
                    SubscriptionEvent::Typing(TypingEvent::Started {
                        context: TypingContext::Dm { peer_key: sender_key.into() },
                        who: sender_key.into(),
                    })
                } else {
                    SubscriptionEvent::Typing(TypingEvent::Stopped {
                        context: TypingContext::Dm { peer_key: sender_key.into() },
                        who: sender_key.into(),
                    })
                }
            }
            Self::FriendRequestAck =>
                SubscriptionEvent::Friend(FriendEvent::RequestAcknowledged { peer_key: sender_key.into() }),
            Self::Unfriend =>
                SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: sender_key.into() }),
            Self::UnfriendAck =>
                SubscriptionEvent::Friend(FriendEvent::RemoveAcknowledged { peer_key: sender_key.into() }),
            Self::ProfileKeyRotated { new_profile_dht_key } =>
                SubscriptionEvent::Friend(FriendEvent::ProfileKeyRotated {
                    peer_key: sender_key.into(), new_profile_dht_key,
                }),
            Self::PresenceUpdate { status, game_info } => {
                let status_str = match status {
                    0 => "online", 1 => "away", 2 => "busy",
                    3 => "offline", 4 => "invisible", _ => "unknown",
                };
                SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
                    peer_key: sender_key.into(),
                    status: status_str.into(),
                    game_name: game_info.map(|g| g.game_name),
                })
            }
        }
    }
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
        DmPayload::Typing { .. } => TypeId::DmTyping,
        DmPayload::FriendRequestAck => TypeId::FriendRequestAck,
        DmPayload::Unfriend => TypeId::Unfriend,
        DmPayload::UnfriendAck => TypeId::UnfriendAck,
        DmPayload::ProfileKeyRotated { .. } => TypeId::ProfileKeyRotated,
        DmPayload::PresenceUpdate { .. } => TypeId::DmPresenceUpdate,
    }
}
