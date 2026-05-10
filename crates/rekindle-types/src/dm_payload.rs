//! DM payload types — ephemeral peer-to-peer signals.
//!
//! DM message content goes through encrypted DhtLog entries, not these types.
//! This enum carries only ephemeral signals: typing, presence, friend lifecycle.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmPayload {
    Typing { typing: bool },
    FriendRequestAck,
    Unfriend,
    UnfriendAck,
    ProfileKeyRotated { new_profile_dht_key: String },
    PresenceUpdate {
        status: u8,
        game_info: Option<GamePresence>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePresence {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    pub server_address: Option<String>,
}

// ── SubscriptionEvent conversion ──────────────────────────────────

use crate::subscription_events::{
    SubscriptionEvent, TypingEvent, TypingContext,
    FriendEvent, PresenceEvent,
};

impl DmPayload {
    /// Convert a DM payload into a `SubscriptionEvent` given sender context.
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
