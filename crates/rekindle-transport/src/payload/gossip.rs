//! Gossip broadcast payload types.
//!
//! The outer [`SignedGossipEnvelope`] carries community routing metadata
//! (community_id, sender_pseudonym, TTL, Lamport timestamp) and an Ed25519
//! signature. The inner [`GossipPayload`] is the deserialized content.

use serde::{Deserialize, Serialize};

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
                _ => {}
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

/// All community control operations.
///
/// Every variant is fully typed — no `serde_json::Value` fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlPayload {
    // ── Member lifecycle ─────────────────────────────────────────
    MemberJoinRequest {
        pseudonym_key: String,
        display_name: String,
        invite_code: Option<String>,
        route_blob: Option<Vec<u8>>,
        prekey_bundle: Option<Vec<u8>>,
        claimed_subkey_index: Option<u32>,
    },
    MemberLeave { pseudonym_key: String },
    JoinAccepted {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
        member_registry_key: Option<String>,
        slot_index: Option<u32>,
        wrapped_slot_seed: Option<Vec<u8>>,
    },
    JoinRejected { reason: String },
    MemberJoined {
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<u32>,
        status: String,
        route_blob: Option<Vec<u8>>,
    },
    MemberRemoved { pseudonym_key: String },

    // ── Moderation ───────────────────────────────────────────────
    Kick { target_pseudonym: String },
    Ban { target_pseudonym: String },
    Unban { target_pseudonym: String },
    TimeoutMember {
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },
    RemoveTimeout { target_pseudonym: String },
    MemberTimedOut {
        pseudonym_key: String,
        timeout_until: Option<u64>,
    },

    // ── Messages ─────────────────────────────────────────────────
    MessageEdited {
        channel_id: String,
        message_id: String,
        new_ciphertext: Vec<u8>,
        mek_generation: u64,
        edited_at: u64,
    },
    MessageDeleted { channel_id: String, message_id: String },

    // ── MEK management ───────────────────────────────────────────
    MekRotated {
        channel_id: Option<String>,
        new_generation: u64,
        rotator_pseudonym: Option<String>,
    },
    RequestMek {
        channel_id: String,
        needed_generation: u64,
        requester_pseudonym: String,
    },
    MekTransfer {
        community_id: String,
        channel_id: Option<String>,
        generation: u64,
        sender_pseudonym: String,
        wrapped_mek: Vec<u8>,
    },

    // ── Roles ────────────────────────────────────────────────────
    MemberRolesChanged { pseudonym_key: String, role_ids: Vec<u32> },
    OnboardingComplete { pseudonym_key: String, role_ids: Vec<u32> },

    // ── Channel permissions ──────────────────────────────────────
    ChannelOverwriteChanged { channel_id: String },

    // ── Reactions & pins ─────────────────────────────────────────
    ReactionAdded { channel_id: String, message_id: String, emoji: String, reactor_pseudonym: String },
    ReactionRemoved { channel_id: String, message_id: String, emoji: String, reactor_pseudonym: String },
    MessagePinned { channel_id: String, message_id: String, pinned_by: String },
    MessageUnpinned { channel_id: String, message_id: String },

    // ── Events ───────────────────────────────────────────────────
    EventCreated { event: CommunityEvent },
    EventUpdated { event: CommunityEvent },
    EventDeleted { event_id: String },
    EventRsvpChanged { event_id: String, pseudonym_key: String, status: String },
    EventReminder { event_id: String, title: String, minutes_until_start: u32 },

    // ── Threads ──────────────────────────────────────────────────
    ThreadCreated { thread: ThreadInfo },
    ThreadMessage {
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        reply_to_id: Option<String>,
    },
    ThreadArchived { thread_id: String, archived: bool },

    // ── Game servers ─────────────────────────────────────────────
    GameServerAdded { server: GameServerInfo },
    GameServerRemoved { server_id: String },

    // ── Governance ───────────────────────────────────────────────
    GovernanceUpdated { governance_key: String, subkey_index: u32, lamport_ts: u64 },

    // ── Voice signaling ──────────────────────────────────────────
    VoiceJoin { channel_id: String, route_blob: Vec<u8> },
    VoiceLeave { channel_id: String },
    VoiceModeSwitch { channel_id: String, mode: String, host_pseudonym: Option<String> },
    VoiceMute { channel_id: String, target_pseudonym: String, muted: bool },
    VoiceDeafen { channel_id: String, target_pseudonym: String, deafened: bool },
    VoiceRoster { channel_id: String, participants: Vec<VoiceParticipant> },

    // ── Admin delegation ─────────────────────────────────────────
    AdminKeypairGrant { wrapped_owner_keypair: Vec<u8>, wrapped_slot_seed: Vec<u8> },
    SlotKeypairGrant { slot_index: u32, segment_index: u32, wrapped_slot_keypair: Vec<u8> },

    // ── Bootstrap (via gossip — not the app_call bootstrap) ──────
    BootstrapRequest { joiner_pseudonym: String, governance_key: String },
    BootstrapResponse {
        governance_entries: Vec<Vec<u8>>,
        member_list: Vec<Vec<u8>>,
        channel_meks: Vec<Vec<u8>>,
        recent_messages: Vec<Vec<u8>>,
        wrapped_owner_keypair: Vec<u8>,
    },

    // ── Sync ─────────────────────────────────────────────────────
    SyncRequest { channel_id: String, since_timestamp: u64 },
    SyncResponse { channel_id: String, messages: Vec<Vec<u8>> },

    // ── System ───────────────────────────────────────────────────
    SystemMessage { body: String, timestamp: u64 },
    RaidAlert { active: bool },
    ChannelLockdown { locked: bool },
    KickedNotification,
    SubmitOnboardingAnswers { answers: Vec<OnboardingAnswer> },
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
