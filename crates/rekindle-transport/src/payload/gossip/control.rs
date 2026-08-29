//! The [`ControlPayload`] enum — all community control operations.
//! Conversion to [`crate::payload::gossip::GossipPayload`]'s
//! `SubscriptionEvent` is split across `into_event_control.rs` and
//! `into_event_control_rest.rs`.

use serde::{Deserialize, Serialize};

use super::{CommunityEvent, GameServerInfo, OnboardingAnswer, ThreadInfo, VoiceParticipant};

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
    MemberLeave {
        pseudonym_key: String,
    },
    JoinAccepted {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
        member_registry_key: Option<String>,
        slot_index: Option<u32>,
        wrapped_slot_seed: Option<Vec<u8>>,
    },
    JoinRejected {
        reason: String,
    },
    MemberJoined {
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<u32>,
        status: String,
        route_blob: Option<Vec<u8>>,
    },
    MemberRemoved {
        pseudonym_key: String,
    },

    // ── Moderation ───────────────────────────────────────────────
    Kick {
        target_pseudonym: String,
    },
    Ban {
        target_pseudonym: String,
    },
    Unban {
        target_pseudonym: String,
    },
    TimeoutMember {
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },
    RemoveTimeout {
        target_pseudonym: String,
    },
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
    MessageDeleted {
        channel_id: String,
        message_id: String,
    },

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
    MemberRolesChanged {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },
    OnboardingComplete {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },

    // ── Channel permissions ──────────────────────────────────────
    ChannelOverwriteChanged {
        channel_id: String,
    },

    // ── Reactions & pins ─────────────────────────────────────────
    ReactionAdded {
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    ReactionRemoved {
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    MessagePinned {
        channel_id: String,
        message_id: String,
        pinned_by: String,
    },
    MessageUnpinned {
        channel_id: String,
        message_id: String,
    },

    // ── Events ───────────────────────────────────────────────────
    EventCreated {
        event: CommunityEvent,
    },
    EventUpdated {
        event: CommunityEvent,
    },
    EventDeleted {
        event_id: String,
    },
    EventRsvpChanged {
        event_id: String,
        pseudonym_key: String,
        status: String,
    },
    EventReminder {
        event_id: String,
        title: String,
        minutes_until_start: u32,
    },

    // ── Threads ──────────────────────────────────────────────────
    ThreadCreated {
        thread: ThreadInfo,
    },
    ThreadMessage {
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        reply_to_id: Option<String>,
    },
    ThreadArchived {
        thread_id: String,
        archived: bool,
    },

    // ── Game servers ─────────────────────────────────────────────
    GameServerAdded {
        server: GameServerInfo,
    },
    GameServerRemoved {
        server_id: String,
    },

    // ── Governance ───────────────────────────────────────────────
    GovernanceUpdated {
        governance_key: String,
        subkey_index: u32,
        lamport_ts: u64,
    },

    // ── Voice signaling ──────────────────────────────────────────
    VoiceJoin {
        channel_id: String,
        route_blob: Vec<u8>,
    },
    VoiceLeave {
        channel_id: String,
    },
    VoiceModeSwitch {
        channel_id: String,
        mode: String,
        host_pseudonym: Option<String>,
    },
    VoiceMute {
        channel_id: String,
        target_pseudonym: String,
        muted: bool,
    },
    VoiceDeafen {
        channel_id: String,
        target_pseudonym: String,
        deafened: bool,
    },
    VoiceRoster {
        channel_id: String,
        participants: Vec<VoiceParticipant>,
    },

    // ── Admin delegation ─────────────────────────────────────────
    AdminKeypairGrant {
        wrapped_owner_keypair: Vec<u8>,
        wrapped_slot_seed: Vec<u8>,
    },
    SlotKeypairGrant {
        slot_index: u32,
        segment_index: u32,
        wrapped_slot_keypair: Vec<u8>,
    },

    // ── Bootstrap (via gossip — not the app_call bootstrap) ──────
    BootstrapRequest {
        joiner_pseudonym: String,
        governance_key: String,
    },
    BootstrapResponse {
        governance_entries: Vec<Vec<u8>>,
        member_list: Vec<Vec<u8>>,
        channel_meks: Vec<Vec<u8>>,
        recent_messages: Vec<Vec<u8>>,
        wrapped_owner_keypair: Vec<u8>,
    },

    // ── Sync ─────────────────────────────────────────────────────
    SyncRequest {
        channel_id: String,
        since_timestamp: u64,
    },
    SyncResponse {
        channel_id: String,
        messages: Vec<Vec<u8>>,
    },

    // ── System ───────────────────────────────────────────────────
    SystemMessage {
        body: String,
        timestamp: u64,
    },
    RaidAlert {
        active: bool,
    },
    ChannelLockdown {
        locked: bool,
    },
    KickedNotification,
    SubmitOnboardingAnswers {
        answers: Vec<OnboardingAnswer>,
    },
}
