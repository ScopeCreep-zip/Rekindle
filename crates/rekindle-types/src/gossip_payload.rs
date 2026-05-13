//! Gossip broadcast payload types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedGossipEnvelope {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub payload_bytes: Vec<u8>,
    pub signature: Vec<u8>,
    pub ttl: u8,
    pub lamport_ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipPayload {
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
    PresenceUpdate {
        pseudonym_key: String,
        status: String,
        game_name: Option<String>,
        game_id: Option<u32>,
        elapsed_seconds: Option<u32>,
        server_address: Option<String>,
        route_blob: Option<Vec<u8>>,
    },
    TypingIndicator {
        channel_id: String,
        pseudonym_key: String,
    },
    Control(ControlPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlPayload {
    MemberJoinRequest { pseudonym_key: String, display_name: String, invite_code: Option<String>, route_blob: Option<Vec<u8>>, prekey_bundle: Option<Vec<u8>>, claimed_subkey_index: Option<u32> },
    MemberLeave { pseudonym_key: String },
    JoinAccepted { mek_encrypted: Vec<u8>, mek_generation: u64, member_registry_key: Option<String>, slot_index: Option<u32>, wrapped_slot_seed: Option<Vec<u8>> },
    JoinRejected { reason: String },
    MemberJoined { pseudonym_key: String, display_name: String, role_ids: Vec<u32>, status: String, route_blob: Option<Vec<u8>> },
    MemberRemoved { pseudonym_key: String },
    Kick { target_pseudonym: String },
    Ban { target_pseudonym: String },
    Unban { target_pseudonym: String },
    TimeoutMember { target_pseudonym: String, duration_seconds: u64, reason: Option<String> },
    RemoveTimeout { target_pseudonym: String },
    MemberTimedOut { pseudonym_key: String, timeout_until: Option<u64> },
    MessageEdited { channel_id: String, message_id: String, new_ciphertext: Vec<u8>, mek_generation: u64, edited_at: u64 },
    MessageDeleted { channel_id: String, message_id: String },
    MekRotated { channel_id: Option<String>, new_generation: u64, rotator_pseudonym: Option<String> },
    RequestMek { channel_id: String, needed_generation: u64, requester_pseudonym: String },
    MekTransfer { community_id: String, channel_id: Option<String>, generation: u64, sender_pseudonym: String, wrapped_mek: Vec<u8> },
    MemberRolesChanged { pseudonym_key: String, role_ids: Vec<u32> },
    OnboardingComplete { pseudonym_key: String, role_ids: Vec<u32> },
    ChannelOverwriteChanged { channel_id: String },
    ReactionAdded { channel_id: String, message_id: String, emoji: String, reactor_pseudonym: String },
    ReactionRemoved { channel_id: String, message_id: String, emoji: String, reactor_pseudonym: String },
    MessagePinned { channel_id: String, message_id: String, pinned_by: String },
    MessageUnpinned { channel_id: String, message_id: String },
    EventCreated { event: CommunityEvent },
    EventUpdated { event: CommunityEvent },
    EventDeleted { event_id: String },
    EventRsvpChanged { event_id: String, pseudonym_key: String, status: String },
    EventReminder { event_id: String, title: String, minutes_until_start: u32 },
    ThreadCreated { thread: ThreadInfo },
    ThreadMessage { thread_id: String, message_id: String, sender_pseudonym: String, ciphertext: Vec<u8>, mek_generation: u64, timestamp: u64, reply_to_id: Option<String> },
    ThreadArchived { thread_id: String, archived: bool },
    GameServerAdded { server: GameServerInfo },
    GameServerRemoved { server_id: String },
    GovernanceUpdated { governance_key: String, subkey_index: u32, lamport_ts: u64 },
    VoiceJoin { channel_id: String, route_blob: Vec<u8> },
    VoiceLeave { channel_id: String },
    VoiceModeSwitch { channel_id: String, mode: String, host_pseudonym: Option<String> },
    VoiceMute { channel_id: String, target_pseudonym: String, muted: bool },
    VoiceDeafen { channel_id: String, target_pseudonym: String, deafened: bool },
    VoiceRoster { channel_id: String, participants: Vec<VoiceParticipant> },
    AdminKeypairGrant { wrapped_owner_keypair: Vec<u8>, wrapped_slot_seed: Vec<u8> },
    SlotKeypairGrant { slot_index: u32, segment_index: u32, wrapped_slot_keypair: Vec<u8> },
    BootstrapRequest { joiner_pseudonym: String, governance_key: String },
    BootstrapResponse { governance_entries: Vec<Vec<u8>>, member_list: Vec<Vec<u8>>, channel_meks: Vec<Vec<u8>>, recent_messages: Vec<Vec<u8>>, wrapped_owner_keypair: Vec<u8> },
    SyncRequest { channel_id: String, since_timestamp: u64 },
    SyncResponse { channel_id: String, messages: Vec<Vec<u8>> },
    SystemMessage { body: String, timestamp: u64 },
    RaidAlert { active: bool },
    ChannelLockdown { locked: bool },
    KickedNotification,
    SubmitOnboardingAnswers { answers: Vec<OnboardingAnswer> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityEvent {
    pub id: String, pub title: String, pub description: String,
    pub creator_pseudonym: String, pub start_time: u64, pub end_time: Option<u64>,
    pub channel_id: Option<String>, pub max_attendees: Option<u32>,
    pub created_at: u64, pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: String, pub channel_id: String, pub name: String,
    pub starter_message_id: String, pub creator_pseudonym: String,
    pub created_at: u64, pub archived: bool, pub auto_archive_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameServerInfo {
    pub id: String, pub game_id: String, pub label: String,
    pub address: String, pub added_by: String, pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceParticipant {
    pub pseudonym_key: String, pub route_blob: Vec<u8>,
    pub muted: bool, pub deafened: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingAnswer {
    pub question_id: String, pub selected_options: Vec<String>,
}

// ── SubscriptionEvent conversions ──────────────────────────────────

use crate::subscription_events::{
    SubscriptionEvent, ChannelMessageEvent,
    TypingEvent, TypingContext,
    PresenceEvent, MembershipEvent,
    CryptoEvent, GovernanceEvent, SocialEvent,
    VoiceEvent, SystemEvent,
};

impl GossipPayload {
    pub fn into_event(self, community: &str, sender: &str) -> SubscriptionEvent {
        let c = || community.to_string();
        let s = || sender.to_string();
        match self {
            Self::MessageNotification { channel_id, message_id, sequence, timestamp, .. } =>
                SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
                    community: c(), channel: channel_id, message_id,
                    sender_pseudonym: s(), sequence, timestamp,
                    body: None, reply_to_sequence: None, is_self: false, client_msg_id: None,
                }),
            Self::TypingIndicator { channel_id, pseudonym_key } =>
                SubscriptionEvent::Typing(TypingEvent::Started {
                    context: TypingContext::Channel { community: c(), channel: channel_id },
                    who: pseudonym_key,
                }),
            Self::PresenceUpdate { pseudonym_key, status, game_name, game_id, .. } =>
                SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
                    community: c(), pseudonym: pseudonym_key, status, game_name, game_id,
                }),
            Self::Control(ctrl) => ctrl.into_event(community, sender),
        }
    }
}

impl ControlPayload {
    pub fn into_event(self, community: &str, sender: &str) -> SubscriptionEvent {
        let c = || community.to_string();
        let s = || sender.to_string();
        match self {
            Self::MemberJoinRequest { pseudonym_key, display_name, invite_code, .. } =>
                SubscriptionEvent::Membership(MembershipEvent::JoinRequested { community: c(), pseudonym: pseudonym_key, display_name, has_invite: invite_code.is_some() }),
            Self::MemberLeave { pseudonym_key } =>
                SubscriptionEvent::Membership(MembershipEvent::Left { community: c(), pseudonym: pseudonym_key }),
            Self::JoinAccepted { mek_generation, slot_index, .. } =>
                SubscriptionEvent::Membership(MembershipEvent::JoinAccepted { community: c(), mek_generation, slot_index }),
            Self::JoinRejected { reason } =>
                SubscriptionEvent::Membership(MembershipEvent::JoinRejected { community: c(), reason }),
            Self::MemberJoined { pseudonym_key, display_name, role_ids, .. } =>
                SubscriptionEvent::Membership(MembershipEvent::Joined { community: c(), pseudonym: pseudonym_key, display_name, role_ids }),
            Self::MemberRemoved { pseudonym_key } =>
                SubscriptionEvent::Membership(MembershipEvent::Removed { community: c(), pseudonym: pseudonym_key }),
            Self::Kick { target_pseudonym } =>
                SubscriptionEvent::Membership(MembershipEvent::Kicked { community: c(), target_pseudonym }),
            Self::Ban { target_pseudonym } =>
                SubscriptionEvent::Membership(MembershipEvent::Banned { community: c(), target_pseudonym }),
            Self::Unban { target_pseudonym } =>
                SubscriptionEvent::Membership(MembershipEvent::Unbanned { community: c(), target_pseudonym }),
            Self::TimeoutMember { target_pseudonym, duration_seconds, reason } =>
                SubscriptionEvent::Membership(MembershipEvent::TimedOut { community: c(), target_pseudonym, duration_seconds, reason }),
            Self::RemoveTimeout { target_pseudonym } =>
                SubscriptionEvent::Membership(MembershipEvent::TimeoutRemoved { community: c(), target_pseudonym }),
            Self::MemberTimedOut { pseudonym_key, timeout_until } =>
                SubscriptionEvent::Membership(MembershipEvent::TimeoutStatusChanged { community: c(), pseudonym: pseudonym_key, timeout_until }),
            Self::MessageEdited { channel_id, message_id, edited_at, .. } =>
                SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited { community: c(), channel: channel_id, message_id, edited_at, body: None }),
            Self::MessageDeleted { channel_id, message_id } =>
                SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted { community: c(), channel: channel_id, message_id }),
            Self::MekRotated { channel_id, new_generation, rotator_pseudonym } =>
                SubscriptionEvent::Crypto(CryptoEvent::MekRotated { community: c(), channel: channel_id, generation: new_generation, rotator_pseudonym }),
            Self::RequestMek { channel_id, needed_generation, requester_pseudonym } =>
                SubscriptionEvent::Crypto(CryptoEvent::MekRequested { community: c(), channel: channel_id, needed_generation, requester_pseudonym }),
            Self::MekTransfer { community_id, channel_id, generation, sender_pseudonym, .. } =>
                SubscriptionEvent::Crypto(CryptoEvent::MekTransferred { community: community_id, channel: channel_id, generation, sender_pseudonym }),
            Self::MemberRolesChanged { pseudonym_key, role_ids } =>
                SubscriptionEvent::Membership(MembershipEvent::RolesChanged { community: c(), pseudonym: pseudonym_key, role_ids }),
            Self::OnboardingComplete { pseudonym_key, role_ids } =>
                SubscriptionEvent::Membership(MembershipEvent::OnboardingCompleted { community: c(), pseudonym: pseudonym_key, role_ids }),
            Self::SubmitOnboardingAnswers { answers } =>
                SubscriptionEvent::Membership(MembershipEvent::OnboardingAnswersSubmitted { community: c(), sender_pseudonym: s(), answer_count: answers.len() }),
            Self::ChannelOverwriteChanged { channel_id } =>
                SubscriptionEvent::Governance(GovernanceEvent::ChannelPermissionsChanged { community: c(), channel: channel_id }),
            Self::ReactionAdded { channel_id, message_id, emoji, reactor_pseudonym } =>
                SubscriptionEvent::Social(SocialEvent::ReactionAdded { community: c(), channel: channel_id, message_id, emoji, reactor_pseudonym }),
            Self::ReactionRemoved { channel_id, message_id, emoji, reactor_pseudonym } =>
                SubscriptionEvent::Social(SocialEvent::ReactionRemoved { community: c(), channel: channel_id, message_id, emoji, reactor_pseudonym }),
            Self::MessagePinned { channel_id, message_id, pinned_by } =>
                SubscriptionEvent::Social(SocialEvent::MessagePinned { community: c(), channel: channel_id, message_id, pinned_by }),
            Self::MessageUnpinned { channel_id, message_id } =>
                SubscriptionEvent::Social(SocialEvent::MessageUnpinned { community: c(), channel: channel_id, message_id }),
            Self::EventCreated { event } =>
                SubscriptionEvent::Social(SocialEvent::EventCreated { community: c(), event_id: event.id, title: event.title, start_time: event.start_time }),
            Self::EventUpdated { event } =>
                SubscriptionEvent::Social(SocialEvent::EventUpdated { community: c(), event_id: event.id, title: event.title }),
            Self::EventDeleted { event_id } =>
                SubscriptionEvent::Social(SocialEvent::EventDeleted { community: c(), event_id }),
            Self::EventRsvpChanged { event_id, pseudonym_key, status } =>
                SubscriptionEvent::Social(SocialEvent::EventRsvpChanged { community: c(), event_id, pseudonym: pseudonym_key, rsvp_status: status }),
            Self::EventReminder { event_id, title, minutes_until_start } =>
                SubscriptionEvent::Social(SocialEvent::EventReminder { community: c(), event_id, title, minutes_until_start }),
            Self::ThreadCreated { thread } =>
                SubscriptionEvent::Social(SocialEvent::ThreadCreated { community: c(), channel: thread.channel_id, thread_id: thread.id, thread_name: thread.name, creator_pseudonym: thread.creator_pseudonym }),
            Self::ThreadMessage { thread_id, message_id, sender_pseudonym, timestamp, .. } =>
                SubscriptionEvent::Social(SocialEvent::ThreadMessagePosted { community: c(), thread_id, message_id, sender_pseudonym, timestamp }),
            Self::ThreadArchived { thread_id, archived } =>
                SubscriptionEvent::Social(SocialEvent::ThreadArchiveChanged { community: c(), thread_id, archived }),
            Self::GameServerAdded { server } =>
                SubscriptionEvent::Social(SocialEvent::GameServerAdded { community: c(), server_id: server.id, game_id: server.game_id, label: server.label }),
            Self::GameServerRemoved { server_id } =>
                SubscriptionEvent::Social(SocialEvent::GameServerRemoved { community: c(), server_id }),
            Self::GovernanceUpdated { subkey_index, lamport_ts, .. } =>
                SubscriptionEvent::Governance(GovernanceEvent::GovernanceSubkeyUpdated { community: c(), subkey_index, lamport_ts }),
            Self::VoiceJoin { channel_id, .. } =>
                SubscriptionEvent::Voice(VoiceEvent::Joined { community: c(), channel: channel_id, pseudonym: s() }),
            Self::VoiceLeave { channel_id } =>
                SubscriptionEvent::Voice(VoiceEvent::Left { community: c(), channel: channel_id, pseudonym: s() }),
            Self::VoiceModeSwitch { channel_id, mode, host_pseudonym } =>
                SubscriptionEvent::Voice(VoiceEvent::ModeChanged { community: c(), channel: channel_id, mode, host_pseudonym }),
            Self::VoiceMute { channel_id, target_pseudonym, muted } =>
                SubscriptionEvent::Voice(VoiceEvent::MuteChanged { community: c(), channel: channel_id, target_pseudonym, muted }),
            Self::VoiceDeafen { channel_id, target_pseudonym, deafened } =>
                SubscriptionEvent::Voice(VoiceEvent::DeafenChanged { community: c(), channel: channel_id, target_pseudonym, deafened }),
            Self::VoiceRoster { channel_id, participants } =>
                SubscriptionEvent::Voice(VoiceEvent::RosterUpdated { community: c(), channel: channel_id, participant_count: participants.len() }),
            Self::AdminKeypairGrant { .. } =>
                SubscriptionEvent::Crypto(CryptoEvent::AdminKeypairGranted { community: c() }),
            Self::SlotKeypairGrant { slot_index, segment_index, .. } =>
                SubscriptionEvent::Crypto(CryptoEvent::SlotKeypairGranted { community: c(), slot_index, segment_index }),
            Self::BootstrapRequest { joiner_pseudonym, .. } =>
                SubscriptionEvent::System(SystemEvent::BootstrapRequested { community: c(), joiner_pseudonym }),
            Self::BootstrapResponse { .. } =>
                SubscriptionEvent::System(SystemEvent::BootstrapReceived { community: c() }),
            Self::SyncRequest { channel_id, since_timestamp } =>
                SubscriptionEvent::System(SystemEvent::SyncRequested { community: c(), channel: channel_id, since_timestamp }),
            Self::SyncResponse { channel_id, messages } =>
                SubscriptionEvent::System(SystemEvent::SyncReceived { community: c(), channel: channel_id, message_count: messages.len() }),
            Self::SystemMessage { body, timestamp } =>
                SubscriptionEvent::System(SystemEvent::Announcement { community: Some(c()), body, timestamp }),
            Self::RaidAlert { active } =>
                SubscriptionEvent::System(SystemEvent::RaidAlert { community: c(), active }),
            Self::ChannelLockdown { locked } =>
                SubscriptionEvent::System(SystemEvent::ChannelLockdown { community: c(), locked }),
            Self::KickedNotification =>
                SubscriptionEvent::System(SystemEvent::Kicked { community: c() }),
        }
    }
}
