//! IPC protocol definitions: Request, Response, and Event enums.
//!
//! These are the typed payloads carried inside `Message<T>`. The bus server
//! routes based on the `Message` envelope; the payload is opaque to routing.
//!
//! Every `IpcRequest` variant maps 1:1 to a `rekindle_transport::operations::*`
//! function or a `QueryEngine` method. There are no catch-all variants — the
//! match in `daemon::dispatch` is exhaustive without a wildcard arm.
//!
//! [RC-16] `IpcRequest::Unlock` and `IdentityCreate` contain secrets — their
//! `Debug` impls redact sensitive fields.


use serde::{Deserialize, Serialize};

use super::message::AgentType;

// ── Subscription filter (re-exported from rekindle-types) ──────────────

pub use rekindle_types::subscription_events::{
    SubscriptionFilter, EventCategory, MAX_FILTERS_PER_CONNECTION,
};

// ── IPC Request ─────────────────────────────────────────────────────────

/// Frontend → Daemon request.
///
/// Every variant is explicitly matched in `daemon::dispatch::dispatch()`.
/// No wildcard arm — adding a variant here forces a handler implementation.
///
/// Variant naming: `{Domain}{Verb}` — e.g., `ChannelCreate`, `FriendAdd`.
/// This convention makes the exhaustive match self-documenting.
///
/// [RC-16] Variants containing secrets (`Unlock`, `IdentityCreate`) have
/// custom `Debug` impls that redact sensitive fields.
#[derive(Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    // ── Lifecycle ──────────────────────────────────────────────────
    /// Unlock the daemon (load signing key, resume session, transition to Operational).
    Unlock { passphrase: String },
    /// Lock the daemon (zeroize signing key, transition to Locked).
    Lock,
    /// Query daemon status (always available, any state).
    /// Returns `StatusSnapshot` with compact status, subscription health,
    /// and full diagnostic checks. Renderers decide display depth.
    Status,
    /// Graceful shutdown: drain connections, stop transport, exit.
    /// Returns Ok before the process exits so the client gets confirmation.
    Shutdown,

    // ── Identity ──────────────────────────────────────────────────
    /// Create a new identity (init ceremony). The daemon generates the keypair,
    /// creates DHT records, stores secrets in the OS keyring, and persists the session.
    IdentityCreate { display_name: String },
    /// Show local identity (pubkey, display name, DHT keys).
    IdentityShow,
    /// Export identity metadata (daemon returns data, CLI writes file).
    IdentityExport,
    /// Rotate the Ed25519 identity keypair. Notifies all friends.
    IdentityRotate,
    /// Destroy local identity (requires typed confirmation).
    IdentityDestroy { confirmation: String },
    /// Factory reset: delete identity, session, keyring, Veilid storage, config.
    IdentityWipe { confirmation: String },
    /// Export identity with passphrase-based encryption (Argon2id + AES-256-GCM).
    /// Daemon encrypts, returns base64-encoded ciphertext.
    IdentityExportEncrypted { passphrase: String },
    /// Import identity from encrypted bundle. Daemon decrypts with passphrase.
    IdentityImportEncrypted { passphrase: String, data: String },
    /// Import identity from plaintext JSON bundle.
    IdentityImport { data: String },

    // ── Friends ───────────────────────────────────────────────────
    /// Send a friend request to a target (profile DHT key).
    FriendAdd { target_profile_key: String, message: String },
    /// Accept a pending friend request.
    FriendAccept { public_key: String },
    /// Reject a pending friend request.
    FriendReject { public_key: String },
    /// Remove a friend.
    FriendRemove { public_key: String },
    /// List all friends with resolved display names and presence.
    FriendList,
    /// List pending inbound friend requests.
    FriendRequests,

    // ── Communities ───────────────────────────────────────────────
    /// Create a new community.
    CommunityCreate { name: String, description: String },
    /// Join a community via governance key or invite code.
    CommunityJoin { invite: String },
    /// Leave a community.
    CommunityLeave { governance_key: String },
    /// List all joined communities.
    CommunityList,
    /// Get detailed community info (channels, roles, members).
    CommunityInfo { governance_key: String },
    /// Approve a pending member from the waiting room.
    CommunityApprove { governance_key: String, member_pseudonym: String },
    /// Reject a pending member from the waiting room.
    CommunityReject { governance_key: String, member_pseudonym: String, reason: String },
    /// List pending join requests for a community.
    CommunityPendingMembers { governance_key: String },
    /// Transfer community ownership to a new owner.
    CommunityTransferOwnership { governance_key: String, new_owner_pseudonym: String },

    // ── Channels ──────────────────────────────────────────────────
    /// List channels in a community.
    ChannelList { community: String },
    /// Create a new channel in a community.
    ChannelCreate {
        community: String,
        name: String,
        kind: String,
        category: Option<String>,
        topic: Option<String>,
        slowmode_seconds: u32,
    },
    /// Delete a channel.
    ChannelDelete { community: String, channel_id: String },
    /// Update channel properties.
    ChannelUpdate {
        community: String,
        channel_id: String,
        name: Option<String>,
        topic: Option<String>,
        slowmode_seconds: Option<u32>,
    },
    /// Send a message to a channel.
    ChannelSend {
        community: String,
        channel: String,
        body: String,
        reply_to: Option<u64>,
    },
    /// Send a typing indicator to a community channel.
    ChannelTyping { community: String, channel: String },
    /// Get channel message history.
    ChannelHistory {
        community: String,
        channel: String,
        limit: u32,
    },
    /// Edit a message in a channel (own messages only).
    MessageEdit {
        community: String,
        channel: String,
        message_id: String,
        new_body: String,
    },
    /// Delete a message in a channel.
    MessageDelete {
        community: String,
        channel: String,
        message_id: String,
    },

    // ── DMs ───────────────────────────────────────────────────────
    /// Send a direct message.
    DmSend { peer_key: String, body: String },
    /// Send a typing indicator to a peer.
    DmTyping { peer_key: String, typing: bool },
    /// List DM inbox.
    DmInbox { limit: u32 },
    /// Load message history for a single DM conversation.
    DmThread { peer_key: String, limit: u32 },

    // ── Subscriptions ─────────────────────────────────────────────
    /// Subscribe to events matching filters.
    Subscribe { filters: Vec<SubscriptionFilter> },
    /// Unsubscribe from events matching filters.
    Unsubscribe { filters: Vec<SubscriptionFilter> },
    /// Mark a context as read — clears daemon-side unread counter and emits UnreadChanged(0).
    MarkRead { context: ReadContext },

    // ── Keys / MEK ───────────────────────────────────────────────
    /// List cached MEKs for a community.
    MekList { community: String },
    /// Rotate MEK for a channel.
    MekRotate { community: String, channel: String },
    /// Request a MEK from community peers (gossip broadcast).
    MekRequest {
        community: String,
        channel: String,
        generation: u64,
    },
    /// Replenish prekeys and publish to profile DHT.
    PrekeyReplenish,

    // ── Presence ──────────────────────────────────────────────────
    /// Set presence status (online, away, busy, invisible).
    PresenceSet { status: String, message: Option<String> },
    /// Set game presence info.
    GamePresenceSet {
        game_name: String,
        game_id: Option<u32>,
        elapsed_seconds: u32,
        server_address: Option<String>,
    },
    /// Clear game presence.
    GamePresenceClear,

    // ── Roles ─────────────────────────────────────────────────────
    /// List all roles in a community.
    RoleList { community: String },
    /// Create a new role.
    RoleCreate {
        community: String,
        name: String,
        permissions: u64,
        color: u32,
        position: i32,
    },
    /// Update a role's properties.
    RoleUpdate {
        community: String,
        role_id: u32,
        name: Option<String>,
        permissions: Option<u64>,
        color: Option<u32>,
    },
    /// Delete a role.
    RoleDelete { community: String, role_id: u32 },
    /// Assign a role to a member.
    RoleAssign {
        community: String,
        member_pseudonym: String,
        role_id: u32,
    },
    /// Remove a role from a member.
    RoleUnassign {
        community: String,
        member_pseudonym: String,
        role_id: u32,
    },

    // ── Moderation ────────────────────────────────────────────────
    /// Kick a member (sync gossip broadcast).
    Kick { community: String, target_pseudonym: String },
    /// Ban a member (persist to governance + gossip broadcast).
    Ban {
        community: String,
        target_pseudonym: String,
        reason: Option<String>,
    },
    /// Unban a member.
    Unban { community: String, target_pseudonym: String },
    /// Timeout a member for a duration.
    Timeout {
        community: String,
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },
    /// List all active bans in a community.
    BanList { community: String },

    // ── Invites ───────────────────────────────────────────────────
    /// Create a community invite.
    InviteCreate {
        community: String,
        max_uses: u32,
        expires_seconds: Option<u64>,
    },
    /// List active invites for a community.
    InviteList { community: String },
    /// Revoke an invite by code.
    InviteRevoke { community: String, invite_code: String },

    // ── Social ────────────────────────────────────────────────────
    /// Add a reaction to a channel message.
    ReactionAdd { community: String, channel: String, message_id: String, emoji: String },
    /// Remove a reaction from a channel message.
    ReactionRemove { community: String, channel: String, message_id: String, emoji: String },
    /// Pin a message in a channel.
    PinAdd { community: String, channel: String, message_id: String },
    /// Unpin a message in a channel.
    PinRemove { community: String, channel: String, message_id: String },
    /// Create a community event.
    EventCreate {
        community: String, title: String, description: String,
        start_time: u64, end_time: Option<u64>,
        channel_id: Option<String>, max_attendees: Option<u32>,
    },
    /// Update a community event.
    EventUpdate {
        community: String, event_id: String, title: String, description: String,
        start_time: u64, end_time: Option<u64>, max_attendees: Option<u32>,
    },
    /// Delete a community event.
    EventDelete { community: String, event_id: String },
    /// RSVP to a community event.
    EventRsvp { community: String, event_id: String, status: String },
    /// Broadcast an event reminder.
    EventRemind { community: String, event_id: String, title: String, minutes_until: u32 },
    /// Create a thread on a channel message.
    ThreadCreate { community: String, channel: String, parent_message_id: String, title: String, auto_archive_seconds: u32 },
    /// Post a message to a thread.
    ThreadMessage { community: String, thread_id: String, ciphertext: Vec<u8>, mek_generation: u64, reply_to_id: Option<String> },
    /// Archive or unarchive a thread.
    ThreadArchive { community: String, thread_id: String, archived: bool },
    /// Add a game server to the community.
    GameServerAdd { community: String, game_id: String, label: String, address: String },
    /// Remove a game server from the community.
    GameServerRemove { community: String, server_id: String },

    // ── System ───────────────────────────────────────────────────
    /// Broadcast a system announcement to all community members.
    SystemAnnounce { community: String, body: String },
    /// Toggle raid alert mode.
    RaidAlert { community: String, active: bool },
    /// Toggle community lockdown (non-operator send block).
    LockdownToggle { community: String, locked: bool },
    /// Notify a kicked member (point-to-point).
    KickNotify { community: String, target_pseudonym: String },
    /// Request bootstrap data from operator.
    BootstrapRequest { community: String },
    /// Send bootstrap response to a joiner (operator only).
    BootstrapRespond {
        community: String, target_pseudonym: String,
        governance_entries: Vec<Vec<u8>>, member_list: Vec<Vec<u8>>,
        channel_meks: Vec<Vec<u8>>, recent_messages: Vec<Vec<u8>>,
        wrapped_owner_keypair: Vec<u8>,
    },
    /// Request channel history sync.
    SyncRequest { community: String, channel_id: String, since_timestamp: u64 },
    /// Respond to a sync request with message history.
    SyncRespond { community: String, target_pseudonym: String, channel_id: String, messages: Vec<Vec<u8>> },

    // ── Voice ─────────────────────────────────────────────────────
    /// Join a voice channel.
    VoiceJoin {
        community: String,
        channel: String,
        muted: bool,
        deafened: bool,
    },
    /// Leave the current voice session.
    VoiceLeave,
    /// Toggle self-mute in the active voice session.
    VoiceMute { muted: bool },
    /// Toggle self-deafen in the active voice session.
    VoiceDeafen { deafened: bool },

    // ── Network / Node ────────────────────────────────────────────
    /// Get detailed network status (peers, routes, circuits).
    NetworkStatus,
    /// Get peer snapshot for display.
    NetworkPeers,

    // ── Agent Management ──────────────────────────────────────────
    /// Register as a named agent with declared capabilities.
    AgentRegister {
        name: String,
        agent_type: AgentType,
        capabilities: Vec<String>,
    },
    /// Revoke an agent's registration.
    AgentRevoke { name: String },
    /// Reload authorization policy from disk.
    PolicyReload,
}

/// [RC-16] Custom Debug that redacts secrets in Unlock and IdentityCreate.
/// All other variants display their fields normally. Body content is shown
/// as length only to avoid logging message text.
impl std::fmt::Debug for IpcRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Secret-bearing variants — redact
            Self::Unlock { .. } => f.debug_struct("Unlock")
                .field("passphrase", &"***REDACTED***")
                .finish(),

            // Variants with message bodies — show length only
            Self::ChannelSend { community, channel, body, reply_to } => f.debug_struct("ChannelSend")
                .field("community", community).field("channel", channel)
                .field("body_len", &body.len()).field("reply_to", reply_to).finish(),
            Self::DmSend { peer_key, body } => f.debug_struct("DmSend")
                .field("peer_key", peer_key).field("body_len", &body.len()).finish(),

            // Everything else — derive-style display
            Self::Lock => write!(f, "Lock"),
            Self::Status => write!(f, "Status"),
            Self::IdentityCreate { display_name } => f.debug_struct("IdentityCreate").field("display_name", display_name).finish(),
            Self::IdentityShow => write!(f, "IdentityShow"),
            Self::IdentityExport => write!(f, "IdentityExport"),
            Self::IdentityRotate => write!(f, "IdentityRotate"),
            Self::IdentityDestroy { confirmation } => f.debug_struct("IdentityDestroy").field("confirmation", confirmation).finish(),
            Self::IdentityWipe { confirmation } => f.debug_struct("IdentityWipe").field("confirmation", confirmation).finish(),
            Self::IdentityExportEncrypted { .. } => f.debug_struct("IdentityExportEncrypted")
                .field("passphrase", &"***REDACTED***").finish(),
            Self::IdentityImportEncrypted { data, .. } => f.debug_struct("IdentityImportEncrypted")
                .field("passphrase", &"***REDACTED***").field("data_len", &data.len()).finish(),
            Self::IdentityImport { data } => f.debug_struct("IdentityImport")
                .field("data_len", &data.len()).finish(),
            Self::FriendAdd { target_profile_key, message } => f.debug_struct("FriendAdd").field("target_profile_key", target_profile_key).field("message", message).finish(),
            Self::FriendAccept { public_key } => f.debug_struct("FriendAccept").field("public_key", public_key).finish(),
            Self::FriendReject { public_key } => f.debug_struct("FriendReject").field("public_key", public_key).finish(),
            Self::FriendRemove { public_key } => f.debug_struct("FriendRemove").field("public_key", public_key).finish(),
            Self::FriendList => write!(f, "FriendList"),
            Self::FriendRequests => write!(f, "FriendRequests"),
            Self::CommunityCreate { name, description } => f.debug_struct("CommunityCreate").field("name", name).field("description", description).finish(),
            Self::CommunityJoin { invite } => f.debug_struct("CommunityJoin").field("invite", invite).finish(),
            Self::CommunityLeave { governance_key } => f.debug_struct("CommunityLeave").field("governance_key", governance_key).finish(),
            Self::CommunityList => write!(f, "CommunityList"),
            Self::CommunityInfo { governance_key } => f.debug_struct("CommunityInfo").field("governance_key", governance_key).finish(),
            Self::CommunityApprove { governance_key, member_pseudonym } => f.debug_struct("CommunityApprove")
                .field("governance_key", governance_key).field("member_pseudonym", member_pseudonym).finish(),
            Self::CommunityReject { governance_key, member_pseudonym, reason } => f.debug_struct("CommunityReject")
                .field("governance_key", governance_key).field("member_pseudonym", member_pseudonym).field("reason", reason).finish(),
            Self::CommunityPendingMembers { governance_key } => f.debug_struct("CommunityPendingMembers").field("governance_key", governance_key).finish(),
            Self::CommunityTransferOwnership { governance_key, new_owner_pseudonym } => f.debug_struct("CommunityTransferOwnership")
                .field("governance_key", governance_key).field("new_owner_pseudonym", new_owner_pseudonym).finish(),
            Self::ChannelList { community } => f.debug_struct("ChannelList").field("community", community).finish(),
            Self::ChannelCreate { community, name, kind, category, topic, slowmode_seconds } => f.debug_struct("ChannelCreate")
                .field("community", community).field("name", name).field("kind", kind)
                .field("category", category).field("topic", topic).field("slowmode_seconds", slowmode_seconds).finish(),
            Self::ChannelDelete { community, channel_id } => f.debug_struct("ChannelDelete").field("community", community).field("channel_id", channel_id).finish(),
            Self::ChannelUpdate { community, channel_id, name, topic, slowmode_seconds } => f.debug_struct("ChannelUpdate")
                .field("community", community).field("channel_id", channel_id)
                .field("name", name).field("topic", topic).field("slowmode_seconds", slowmode_seconds).finish(),
            Self::ChannelHistory { community, channel, limit } => f.debug_struct("ChannelHistory").field("community", community).field("channel", channel).field("limit", limit).finish(),
            Self::ChannelTyping { community, channel } => f.debug_struct("ChannelTyping").field("community", community).field("channel", channel).finish(),
            Self::MessageEdit { community, channel, message_id, new_body } => f.debug_struct("MessageEdit")
                .field("community", community).field("channel", channel).field("message_id", message_id).field("body_len", &new_body.len()).finish(),
            Self::MessageDelete { community, channel, message_id } => f.debug_struct("MessageDelete")
                .field("community", community).field("channel", channel).field("message_id", message_id).finish(),
            Self::DmTyping { peer_key, typing } => f.debug_struct("DmTyping").field("peer_key", peer_key).field("typing", typing).finish(),
            Self::DmInbox { limit } => f.debug_struct("DmInbox").field("limit", limit).finish(),
            Self::DmThread { peer_key, limit } => f.debug_struct("DmThread").field("peer_key", peer_key).field("limit", limit).finish(),
            Self::Subscribe { filters } => f.debug_struct("Subscribe").field("filter_count", &filters.len()).finish(),
            Self::Unsubscribe { filters } => f.debug_struct("Unsubscribe").field("filter_count", &filters.len()).finish(),
            Self::MarkRead { context } => f.debug_struct("MarkRead").field("context", context).finish(),
            Self::MekList { community } => f.debug_struct("MekList").field("community", community).finish(),
            Self::MekRotate { community, channel } => f.debug_struct("MekRotate").field("community", community).field("channel", channel).finish(),
            Self::MekRequest { community, channel, generation } => f.debug_struct("MekRequest").field("community", community).field("channel", channel).field("generation", generation).finish(),
            Self::PrekeyReplenish => write!(f, "PrekeyReplenish"),
            Self::PresenceSet { status, message } => f.debug_struct("PresenceSet").field("status", status).field("message", message).finish(),
            Self::GamePresenceSet { game_name, game_id, elapsed_seconds, server_address } => f.debug_struct("GamePresenceSet")
                .field("game_name", game_name).field("game_id", game_id)
                .field("elapsed_seconds", elapsed_seconds).field("server_address", server_address).finish(),
            Self::GamePresenceClear => write!(f, "GamePresenceClear"),
            Self::RoleList { community } => f.debug_struct("RoleList").field("community", community).finish(),
            Self::RoleCreate { community, name, permissions, color, position } => f.debug_struct("RoleCreate")
                .field("community", community).field("name", name).field("permissions", permissions)
                .field("color", color).field("position", position).finish(),
            Self::RoleUpdate { community, role_id, name, permissions, color } => f.debug_struct("RoleUpdate")
                .field("community", community).field("role_id", role_id)
                .field("name", name).field("permissions", permissions).field("color", color).finish(),
            Self::RoleDelete { community, role_id } => f.debug_struct("RoleDelete").field("community", community).field("role_id", role_id).finish(),
            Self::RoleAssign { community, member_pseudonym, role_id } => f.debug_struct("RoleAssign")
                .field("community", community).field("member_pseudonym", member_pseudonym).field("role_id", role_id).finish(),
            Self::RoleUnassign { community, member_pseudonym, role_id } => f.debug_struct("RoleUnassign")
                .field("community", community).field("member_pseudonym", member_pseudonym).field("role_id", role_id).finish(),
            Self::Kick { community, target_pseudonym } => f.debug_struct("Kick").field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::Ban { community, target_pseudonym, reason } => f.debug_struct("Ban")
                .field("community", community).field("target_pseudonym", target_pseudonym).field("reason", reason).finish(),
            Self::Unban { community, target_pseudonym } => f.debug_struct("Unban").field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::Timeout { community, target_pseudonym, duration_seconds, reason } => f.debug_struct("Timeout")
                .field("community", community).field("target_pseudonym", target_pseudonym)
                .field("duration_seconds", duration_seconds).field("reason", reason).finish(),
            Self::BanList { community } => f.debug_struct("BanList").field("community", community).finish(),
            Self::InviteCreate { community, max_uses, expires_seconds } => f.debug_struct("InviteCreate")
                .field("community", community).field("max_uses", max_uses).field("expires_seconds", expires_seconds).finish(),
            Self::InviteList { community } => f.debug_struct("InviteList").field("community", community).finish(),
            Self::InviteRevoke { community, invite_code } => f.debug_struct("InviteRevoke").field("community", community).field("invite_code", invite_code).finish(),
            Self::VoiceJoin { community, channel, muted, deafened } => f.debug_struct("VoiceJoin")
                .field("community", community).field("channel", channel).field("muted", muted).field("deafened", deafened).finish(),
            Self::VoiceLeave => write!(f, "VoiceLeave"),
            Self::VoiceMute { muted } => f.debug_struct("VoiceMute").field("muted", muted).finish(),
            Self::VoiceDeafen { deafened } => f.debug_struct("VoiceDeafen").field("deafened", deafened).finish(),
            // Social
            Self::ReactionAdd { community, channel, message_id, emoji } => f.debug_struct("ReactionAdd")
                .field("community", community).field("channel", channel).field("message_id", message_id).field("emoji", emoji).finish(),
            Self::ReactionRemove { community, channel, message_id, emoji } => f.debug_struct("ReactionRemove")
                .field("community", community).field("channel", channel).field("message_id", message_id).field("emoji", emoji).finish(),
            Self::PinAdd { community, channel, message_id } => f.debug_struct("PinAdd")
                .field("community", community).field("channel", channel).field("message_id", message_id).finish(),
            Self::PinRemove { community, channel, message_id } => f.debug_struct("PinRemove")
                .field("community", community).field("channel", channel).field("message_id", message_id).finish(),
            Self::EventCreate { community, title, .. } => f.debug_struct("EventCreate")
                .field("community", community).field("title", title).finish(),
            Self::EventUpdate { community, event_id, title, .. } => f.debug_struct("EventUpdate")
                .field("community", community).field("event_id", event_id).field("title", title).finish(),
            Self::EventDelete { community, event_id } => f.debug_struct("EventDelete")
                .field("community", community).field("event_id", event_id).finish(),
            Self::EventRsvp { community, event_id, status } => f.debug_struct("EventRsvp")
                .field("community", community).field("event_id", event_id).field("status", status).finish(),
            Self::EventRemind { community, event_id, title, minutes_until } => f.debug_struct("EventRemind")
                .field("community", community).field("event_id", event_id).field("title", title).field("minutes_until", minutes_until).finish(),
            Self::ThreadCreate { community, channel, title, .. } => f.debug_struct("ThreadCreate")
                .field("community", community).field("channel", channel).field("title", title).finish(),
            Self::ThreadMessage { community, thread_id, mek_generation, .. } => f.debug_struct("ThreadMessage")
                .field("community", community).field("thread_id", thread_id).field("mek_generation", mek_generation).finish(),
            Self::ThreadArchive { community, thread_id, archived } => f.debug_struct("ThreadArchive")
                .field("community", community).field("thread_id", thread_id).field("archived", archived).finish(),
            Self::GameServerAdd { community, game_id, label, address } => f.debug_struct("GameServerAdd")
                .field("community", community).field("game_id", game_id).field("label", label).field("address", address).finish(),
            Self::GameServerRemove { community, server_id } => f.debug_struct("GameServerRemove")
                .field("community", community).field("server_id", server_id).finish(),
            // System
            Self::SystemAnnounce { community, body } => f.debug_struct("SystemAnnounce")
                .field("community", community).field("body_len", &body.len()).finish(),
            Self::RaidAlert { community, active } => f.debug_struct("RaidAlert")
                .field("community", community).field("active", active).finish(),
            Self::LockdownToggle { community, locked } => f.debug_struct("LockdownToggle")
                .field("community", community).field("locked", locked).finish(),
            Self::KickNotify { community, target_pseudonym } => f.debug_struct("KickNotify")
                .field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::BootstrapRequest { community } => f.debug_struct("BootstrapRequest")
                .field("community", community).finish(),
            Self::BootstrapRespond { community, target_pseudonym, .. } => f.debug_struct("BootstrapRespond")
                .field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::SyncRequest { community, channel_id, since_timestamp } => f.debug_struct("SyncRequest")
                .field("community", community).field("channel_id", channel_id).field("since_timestamp", since_timestamp).finish(),
            Self::SyncRespond { community, target_pseudonym, channel_id, .. } => f.debug_struct("SyncRespond")
                .field("community", community).field("target_pseudonym", target_pseudonym).field("channel_id", channel_id).finish(),
            Self::NetworkStatus => write!(f, "NetworkStatus"),
            Self::NetworkPeers => write!(f, "NetworkPeers"),
            Self::AgentRegister { name, agent_type, capabilities } => f.debug_struct("AgentRegister")
                .field("name", name).field("agent_type", agent_type).field("capabilities", capabilities).finish(),
            Self::AgentRevoke { name } => f.debug_struct("AgentRevoke").field("name", name).finish(),
            Self::PolicyReload => write!(f, "PolicyReload"),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}

// ── Read Context ────────────────────────────────────────────────────────

/// Context for marking a conversation as read. Makes invalid states
/// unrepresentable — exactly one of Channel or Dm, never both/neither.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReadContext {
    Channel { community: String, channel: String },
    Dm { peer: String },
}

// ── IPC Response ────────────────────────────────────────────────────────

/// Daemon → Frontend response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// Success with a JSON value payload.
    Ok(serde_json::Value),
    /// Error with code, message, and optional remediation advice.
    Error {
        code: u32,
        message: String,
        remediation: Option<String>,
    },
    /// Unsolicited push: a subscription event from the three-tier event pipeline.
    /// Delivered to all connected clients via the event push task.
    Event(rekindle_types::subscription_events::SubscriptionEvent),
}

impl IpcResponse {
    /// Create a success response from a serializable value.
    pub fn ok<T: Serialize>(value: &T) -> Self {
        // [RC-2] serde_json::to_value can fail on recursive structures,
        // but our types are flat. If it fails, return an internal error.
        match serde_json::to_value(value) {
            std::result::Result::Ok(v) => Self::Ok(v),
            Err(e) => Self::Error {
                code: 500,
                message: format!("response serialization failed: {e}"),
                remediation: None,
            },
        }
    }

    /// Create an error response.
    #[must_use]
    pub fn error(code: u32, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
            remediation: None,
        }
    }

    /// Create an error response with remediation advice.
    #[must_use]
    pub fn error_with_remediation(
        code: u32,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self::Error {
            code,
            message: message.into(),
            remediation: Some(remediation.into()),
        }
    }
}

// ── Bus Payload ────────────────────────────────────────────────────────

/// Universal wire payload for the IPC bus.
///
/// Every `Message<BusPayload>` on the bus uses this enum so the server
/// can decode every frame with a single type — required because postcard
/// is schema-aware and cannot decode `Message<IpcRequest>` bytes as
/// `Message<IpcResponse>`.
///
/// The server is a pure router: it decodes `Message<BusPayload>`, routes
/// by `correlation_id` for responses, broadcasts for new requests, and
/// stamps `verified_sender_name`. It never inspects the inner variant.
///
/// Participants wrap their payloads:
/// - CLI/TUI clients: `BusPayload::Request(IpcRequest)`
/// - Daemon subscriber: `BusPayload::Response(IpcResponse)`
/// - Subscription delivery: `BusPayload::Event(SubscriptionEvent)`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BusPayload {
    /// Frontend → Daemon request.
    Request(IpcRequest),
    /// Daemon → Frontend response, serialized as JSON bytes.
    ///
    /// `IpcResponse` contains `serde_json::Value` which postcard cannot
    /// handle (postcard requires schema-aware types, not self-describing).
    /// The response is serialized to JSON by the daemon subscriber, carried
    /// as raw bytes through the postcard-encoded bus, and deserialized from
    /// JSON by the client. This is the only type that crosses the postcard/JSON
    /// boundary — all other variants are fully postcard-compatible.
    Response(Vec<u8>),
    /// Daemon → Frontend push event, routed via EventRouter to subscribed connections.
    Event(rekindle_types::subscription_events::SubscriptionEvent),
}
