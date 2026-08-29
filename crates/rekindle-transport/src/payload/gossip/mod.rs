//! Gossip broadcast payload types.
//!
//! The outer [`SignedGossipEnvelope`] carries community routing metadata
//! (community_id, sender_pseudonym, TTL, Lamport timestamp) and an Ed25519
//! signature. The inner [`GossipPayload`] is the deserialized content.

use serde::{Deserialize, Serialize};

mod control;
mod into_event_control;
mod into_event_control_rest;

pub use control::ControlPayload;

use rekindle_types::subscription_events::{
    ChannelMessageEvent, PresenceEvent, SubscriptionEvent, TypingContext, TypingEvent,
};

/// Signed gossip envelope — the wire format for community broadcasts.
///
/// Signature covers `payload_bytes` only. Routing fields (community_id,
/// sender_pseudonym, ttl, lamport_ts) are in the clear for dedup/routing
/// but the payload itself is authenticated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedGossipEnvelope {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub payload_bytes: Vec<u8>,
    pub signature: Vec<u8>,
    pub ttl: u8,
    pub lamport_ts: u64,
}

impl SignedGossipEnvelope {
    /// Compute a dedup key for this envelope.
    ///
    /// For message notifications: use the message_id.
    /// For typing/presence: use a time-bucketed key to collapse rapid updates.
    /// For everything else: BLAKE3 hash of the payload bytes.
    pub fn dedup_key(&self) -> String {
        // Try to extract a deterministic key from the payload
        if let Ok(payload) = postcard::from_bytes::<GossipPayload>(&self.payload_bytes) {
            match &payload {
                GossipPayload::MessageNotification { message_id, .. } => {
                    return message_id.clone();
                }
                GossipPayload::TypingIndicator { channel_id, .. } => {
                    let bucket = rekindle_utils::timestamp_secs() / 5;
                    return format!("typing:{channel_id}:{}:{bucket}", self.sender_pseudonym);
                }
                GossipPayload::PresenceUpdate { .. } => {
                    let bucket = rekindle_utils::timestamp_secs() / 30;
                    return format!("presence:{}:{bucket}", self.sender_pseudonym);
                }
                GossipPayload::Control(_) => {}
            }
        }
        // Fallback: BLAKE3 hash of payload bytes
        let hash = blake3::hash(&self.payload_bytes);
        hex::encode(&hash.as_bytes()[..16])
    }

    /// Whether this envelope carries a private payload that should NOT be forwarded.
    pub fn is_private(&self) -> bool {
        if let Ok(payload) = postcard::from_bytes::<GossipPayload>(&self.payload_bytes) {
            matches!(
                payload,
                GossipPayload::Control(
                    ControlPayload::JoinAccepted { .. }
                        | ControlPayload::JoinRejected { .. }
                        | ControlPayload::SlotKeypairGrant { .. }
                        | ControlPayload::AdminKeypairGrant { .. }
                        | ControlPayload::SyncResponse { .. }
                        | ControlPayload::KickedNotification
                )
            )
        } else {
            false
        }
    }
}

/// Inner gossip payload — the authenticated content of a broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipPayload {
    /// Notification that a new message exists in a channel SMPL record.
    /// Gossip carries the manifest (metadata), not the cargo (ciphertext).
    MessageNotification {
        channel_id: String,
        message_id: String,
        author_pseudonym: String,
        subkey_index: u32,
        lamport_ts: u64,
        sequence: u64,
        content_hash: String,
        timestamp: u64,
    },
    /// Member presence update.
    PresenceUpdate {
        pseudonym_key: String,
        status: String,
        game_name: Option<String>,
        game_id: Option<u32>,
        elapsed_seconds: Option<u32>,
        server_address: Option<String>,
        route_blob: Option<Vec<u8>>,
    },
    /// Typing indicator (ephemeral, not stored).
    TypingIndicator {
        channel_id: String,
        pseudonym_key: String,
    },
    /// A control operation.
    Control(ControlPayload),
}

// ── Supporting types (fully typed, no serde_json::Value) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityEvent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub creator_pseudonym: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<String>,
    pub max_attendees: Option<u32>,
    pub created_at: u64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub starter_message_id: String,
    pub creator_pseudonym: String,
    pub created_at: u64,
    pub archived: bool,
    pub auto_archive_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameServerInfo {
    pub id: String,
    pub game_id: String,
    pub label: String,
    pub address: String,
    pub added_by: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceParticipant {
    pub pseudonym_key: String,
    pub route_blob: Vec<u8>,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingAnswer {
    pub question_id: String,
    pub selected_options: Vec<String>,
}

// ── SubscriptionEvent conversion ───────────────────────────────────────

impl GossipPayload {
    /// Convert a gossip payload into a `SubscriptionEvent` given envelope context.
    ///
    /// This is a pure data transformation — no state mutation, no I/O, no logging.
    /// The compiler enforces exhaustiveness: adding a new `ControlPayload` variant
    /// without a match arm here is a build error.
    pub fn into_event(self, community: &str, sender: &str) -> SubscriptionEvent {
        let c = || community.to_string();
        let s = || sender.to_string();

        match self {
            Self::MessageNotification {
                channel_id,
                message_id,
                sequence,
                timestamp,
                ..
            } => {
                SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
                    community: c(),
                    channel: channel_id,
                    message_id,
                    sender_pseudonym: s(),
                    sequence,
                    timestamp,
                    body: None,              // populated by enrichment stage
                    reply_to_sequence: None, // populated by enrichment stage
                })
            }
            Self::TypingIndicator {
                channel_id,
                pseudonym_key,
            } => SubscriptionEvent::Typing(TypingEvent::Started {
                context: TypingContext::Channel {
                    community: c(),
                    channel: channel_id,
                },
                who: pseudonym_key,
            }),
            Self::PresenceUpdate {
                pseudonym_key,
                status,
                game_name,
                game_id,
                ..
            } => SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
                community: c(),
                pseudonym: pseudonym_key,
                status,
                game_name,
                game_id,
            }),
            Self::Control(ctrl) => ctrl.into_event(community, sender),
        }
    }
}
