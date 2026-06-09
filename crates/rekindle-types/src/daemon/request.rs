//! Daemon request vocabulary — every command the daemon accepts.
//!
//! Every variant maps 1:1 to a `ChatService` method or a daemon lifecycle
//! operation. No catch-all variants — the match in daemon dispatch is
//! exhaustive without a wildcard arm.
//!
//! Variant naming: `{Domain}{Verb}` — e.g., `ChannelCreate`, `FriendAdd`.
//!
//! Secrets (`Unlock`, `IdentityExportEncrypted`) have a custom [`Debug`]
//! impl that redacts sensitive fields. Message bodies are shown as length
//! only to avoid logging user content.

use serde::{Deserialize, Serialize};

use super::AgentType;
use crate::subscription_events::SubscriptionFilter;
use super::response::ReadContext;

#[derive(Clone, Serialize, Deserialize)]
pub enum DaemonRequest {
    // ── Lifecycle ──────────────────────────────────────────────────
    /// Unlock the daemon (passphrase → Argon2id → vault open → transport start).
    Unlock { passphrase: String },
    /// Lock the daemon (zeroize secrets, stop transport, transition to Locked).
    Lock,
    /// Query daemon status (always available, any state).
    Status,
    /// Graceful shutdown: drain connections, stop transport, exit.
    Shutdown,

    // ── Identity ──────────────────────────────────────────────────
    /// Create a new identity (init ceremony).
    IdentityCreate { display_name: String },
    /// Show local identity (pubkey, display name, DHT keys).
    IdentityShow,
    /// Export identity metadata.
    IdentityExport,
    /// Rotate the Ed25519 identity keypair.
    IdentityRotate,
    /// Destroy local identity (requires typed confirmation).
    IdentityDestroy { confirmation: String },
    /// Factory reset: delete identity, session, keyring, Veilid storage, config.
    IdentityWipe { confirmation: String },
    /// Export identity with passphrase-based encryption.
    IdentityExportEncrypted { passphrase: String },
    /// Import identity from encrypted bundle.
    IdentityImportEncrypted { passphrase: String, data: String },
    /// Import identity from plaintext JSON bundle.
    IdentityImport { data: String },

    // ── Friends ───────────────────────────────────────────────────
    /// Send a friend request.
    FriendAdd { target_profile_key: String, message: String },
    /// Accept a pending friend request.
    FriendAccept { public_key: String },
    /// Reject a pending friend request.
    FriendReject { public_key: String },
    /// Remove a friend.
    FriendRemove { public_key: String },
    /// List all friends.
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
    /// Get detailed community info.
    CommunityInfo { governance_key: String },
    /// Approve a pending member.
    CommunityApprove { governance_key: String, member_pseudonym: String },
    /// Reject a pending member.
    CommunityReject { governance_key: String, member_pseudonym: String, reason: String },
    /// List pending join requests.
    CommunityPendingMembers { governance_key: String },
    /// Transfer community ownership.
    CommunityTransferOwnership { governance_key: String, new_owner_pseudonym: String },

    // ── Channels ──────────────────────────────────────────────────
    /// List channels in a community.
    ChannelList { community: String },
    /// Create a channel.
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
        /// Client-generated idempotency key (UUID v7).
        #[serde(default)]
        client_msg_id: Option<String>,
    },
    /// Send a typing indicator.
    ChannelTyping { community: String, channel: String },
    /// Get channel message history.
    ChannelHistory { community: String, channel: String, limit: u32 },
    /// Edit a message (own messages only).
    MessageEdit { community: String, channel: String, message_id: String, new_body: String },
    /// Delete a message.
    MessageDelete { community: String, channel: String, message_id: String },

    // ── DMs ───────────────────────────────────────────────────────
    /// Send a direct message.
    DmSend { peer_key: String, body: String },
    /// Send a DM typing indicator.
    DmTyping { peer_key: String, typing: bool },
    /// List DM inbox.
    DmInbox { limit: u32 },
    /// Load DM thread history.
    DmThread { peer_key: String, limit: u32 },

    // ── Subscriptions ─────────────────────────────────────────────
    /// Subscribe to events matching filters.
    Subscribe { filters: Vec<SubscriptionFilter> },
    /// Unsubscribe from events matching filters.
    Unsubscribe { filters: Vec<SubscriptionFilter> },
    /// Mark a context as read.
    MarkRead { context: ReadContext },

    // ── Keys / MEK ───────────────────────────────────────────────
    /// List cached MEKs for a community.
    MekList { community: String },
    /// Rotate MEK for a channel.
    MekRotate { community: String, channel: String },
    /// Request a MEK from community peers.
    MekRequest { community: String, channel: String, generation: u64 },
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
    /// Create a role.
    RoleCreate {
        community: String,
        name: String,
        permissions: u64,
        color: u32,
        position: i32,
    },
    /// Update a role.
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
    RoleAssign { community: String, member_pseudonym: String, role_id: u32 },
    /// Remove a role from a member.
    RoleUnassign { community: String, member_pseudonym: String, role_id: u32 },

    // ── Moderation ────────────────────────────────────────────────
    /// Kick a member.
    Kick { community: String, target_pseudonym: String },
    /// Ban a member.
    Ban { community: String, target_pseudonym: String, reason: Option<String> },
    /// Unban a member.
    Unban { community: String, target_pseudonym: String },
    /// Timeout a member.
    Timeout {
        community: String,
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },
    /// List all active bans.
    BanList { community: String },

    // ── Invites ───────────────────────────────────────────────────
    /// Create a community invite.
    InviteCreate { community: String, max_uses: u32, expires_seconds: Option<u64> },
    /// List active invites.
    InviteList { community: String },
    /// Revoke an invite.
    InviteRevoke { community: String, invite_code: String },

    // ── Social ────────────────────────────────────────────────────
    /// Add a reaction.
    ReactionAdd { community: String, channel: String, message_id: String, emoji: String },
    /// Remove a reaction.
    ReactionRemove { community: String, channel: String, message_id: String, emoji: String },
    /// Pin a message.
    PinAdd { community: String, channel: String, message_id: String },
    /// Unpin a message.
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
    /// Create a thread.
    ThreadCreate {
        community: String, channel: String, parent_message_id: String,
        title: String, auto_archive_seconds: u32,
    },
    /// Post a message to a thread.
    ThreadMessage {
        community: String, thread_id: String, ciphertext: Vec<u8>,
        mek_generation: u64, reply_to_id: Option<String>,
    },
    /// Archive or unarchive a thread.
    ThreadArchive { community: String, thread_id: String, archived: bool },
    /// Add a game server.
    GameServerAdd { community: String, game_id: String, label: String, address: String },
    /// Remove a game server.
    GameServerRemove { community: String, server_id: String },

    // ── System ───────────────────────────────────────────────────
    /// Broadcast a system announcement.
    SystemAnnounce { community: String, body: String },
    /// Toggle raid alert mode.
    RaidAlert { community: String, active: bool },
    /// Toggle community lockdown.
    LockdownToggle { community: String, locked: bool },
    /// Notify a kicked member.
    KickNotify { community: String, target_pseudonym: String },
    /// Request bootstrap data.
    BootstrapRequest { community: String },
    /// Send bootstrap response.
    BootstrapRespond {
        community: String, target_pseudonym: String,
        governance_entries: Vec<Vec<u8>>, member_list: Vec<Vec<u8>>,
        channel_meks: Vec<Vec<u8>>, recent_messages: Vec<Vec<u8>>,
        wrapped_owner_keypair: Vec<u8>,
    },
    /// Request channel history sync.
    SyncRequest { community: String, channel_id: String, since_timestamp: u64 },
    /// Respond with sync history.
    SyncRespond {
        community: String, target_pseudonym: String,
        channel_id: String, messages: Vec<Vec<u8>>,
    },

    // ── Voice ─────────────────────────────────────────────────────
    /// Join a voice channel.
    VoiceJoin { community: String, channel: String, muted: bool, deafened: bool },
    /// Leave the current voice session.
    VoiceLeave,
    /// Toggle self-mute.
    VoiceMute { muted: bool },
    /// Toggle self-deafen.
    VoiceDeafen { deafened: bool },

    // ── Bulk Transfer ─────────────────────────────────────────────
    /// Initiate a bulk transfer.
    BulkTransferStart {
        transfer_id: String,
        total_size: u64,
        media_type: String,
        digest: String,
        direction: String,
    },
    /// Signal completion of a bulk transfer.
    BulkTransferComplete { transfer_id: String, digest: String, bytes_transferred: u64 },
    /// Cancel an in-progress bulk transfer.
    BulkTransferCancel { transfer_id: String, reason: String },
    /// Query transfer progress.
    BulkTransferStatus { transfer_id: String },

    // ── Network / Node ────────────────────────────────────────────
    /// Get detailed network status.
    NetworkStatus,
    /// Get peer snapshot.
    NetworkPeers,

    // ── Agent Management ──────────────────────────────────────────
    /// Register as a named agent.
    AgentRegister { name: String, agent_type: AgentType, capabilities: Vec<String> },
    /// Revoke an agent's registration.
    AgentRevoke { name: String },
    /// Reload authorization policy from disk.
    PolicyReload,

    // ── Event Journal ─────────────────────────────────────────────
    /// Resume event delivery from a cursor position.
    EventResume { last_seen_seq: Option<u64> },
}

// ── Debug impl — redacts secrets, shows body lengths ────────────────────

impl std::fmt::Debug for DaemonRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unlock { .. } => f.debug_struct("Unlock")
                .field("passphrase", &"***REDACTED***").finish(),
            Self::IdentityExportEncrypted { .. } => f.debug_struct("IdentityExportEncrypted")
                .field("passphrase", &"***REDACTED***").finish(),
            Self::IdentityImportEncrypted { data, .. } => f.debug_struct("IdentityImportEncrypted")
                .field("passphrase", &"***REDACTED***").field("data_len", &data.len()).finish(),

            Self::ChannelSend { community, channel, body, reply_to, client_msg_id } =>
                f.debug_struct("ChannelSend")
                    .field("community", community).field("channel", channel)
                    .field("body_len", &body.len()).field("reply_to", reply_to)
                    .field("client_msg_id", client_msg_id).finish(),
            Self::DmSend { peer_key, body } => f.debug_struct("DmSend")
                .field("peer_key", peer_key).field("body_len", &body.len()).finish(),
            Self::SystemAnnounce { community, body } => f.debug_struct("SystemAnnounce")
                .field("community", community).field("body_len", &body.len()).finish(),
            Self::MessageEdit { community, channel, message_id, new_body } =>
                f.debug_struct("MessageEdit")
                    .field("community", community).field("channel", channel)
                    .field("message_id", message_id).field("body_len", &new_body.len()).finish(),
            Self::IdentityImport { data } => f.debug_struct("IdentityImport")
                .field("data_len", &data.len()).finish(),

            // Everything else — derive-style, no redaction needed.
            Self::Lock => write!(f, "Lock"),
            Self::Status => write!(f, "Status"),
            Self::Shutdown => write!(f, "Shutdown"),
            Self::IdentityCreate { display_name } => f.debug_struct("IdentityCreate").field("display_name", display_name).finish(),
            Self::IdentityShow => write!(f, "IdentityShow"),
            Self::IdentityExport => write!(f, "IdentityExport"),
            Self::IdentityRotate => write!(f, "IdentityRotate"),
            Self::IdentityDestroy { confirmation } => f.debug_struct("IdentityDestroy").field("confirmation", confirmation).finish(),
            Self::IdentityWipe { confirmation } => f.debug_struct("IdentityWipe").field("confirmation", confirmation).finish(),
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
            Self::CommunityApprove { governance_key, member_pseudonym } => f.debug_struct("CommunityApprove").field("governance_key", governance_key).field("member_pseudonym", member_pseudonym).finish(),
            Self::CommunityReject { governance_key, member_pseudonym, reason } => f.debug_struct("CommunityReject").field("governance_key", governance_key).field("member_pseudonym", member_pseudonym).field("reason", reason).finish(),
            Self::CommunityPendingMembers { governance_key } => f.debug_struct("CommunityPendingMembers").field("governance_key", governance_key).finish(),
            Self::CommunityTransferOwnership { governance_key, new_owner_pseudonym } => f.debug_struct("CommunityTransferOwnership").field("governance_key", governance_key).field("new_owner_pseudonym", new_owner_pseudonym).finish(),
            Self::ChannelList { community } => f.debug_struct("ChannelList").field("community", community).finish(),
            Self::ChannelCreate { community, name, kind, category, topic, slowmode_seconds } => f.debug_struct("ChannelCreate").field("community", community).field("name", name).field("kind", kind).field("category", category).field("topic", topic).field("slowmode_seconds", slowmode_seconds).finish(),
            Self::ChannelDelete { community, channel_id } => f.debug_struct("ChannelDelete").field("community", community).field("channel_id", channel_id).finish(),
            Self::ChannelUpdate { community, channel_id, name, topic, slowmode_seconds } => f.debug_struct("ChannelUpdate").field("community", community).field("channel_id", channel_id).field("name", name).field("topic", topic).field("slowmode_seconds", slowmode_seconds).finish(),
            Self::ChannelHistory { community, channel, limit } => f.debug_struct("ChannelHistory").field("community", community).field("channel", channel).field("limit", limit).finish(),
            Self::ChannelTyping { community, channel } => f.debug_struct("ChannelTyping").field("community", community).field("channel", channel).finish(),
            Self::MessageDelete { community, channel, message_id } => f.debug_struct("MessageDelete").field("community", community).field("channel", channel).field("message_id", message_id).finish(),
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
            Self::GamePresenceSet { game_name, game_id, elapsed_seconds, server_address } => f.debug_struct("GamePresenceSet").field("game_name", game_name).field("game_id", game_id).field("elapsed_seconds", elapsed_seconds).field("server_address", server_address).finish(),
            Self::GamePresenceClear => write!(f, "GamePresenceClear"),
            Self::RoleList { community } => f.debug_struct("RoleList").field("community", community).finish(),
            Self::RoleCreate { community, name, permissions, color, position } => f.debug_struct("RoleCreate").field("community", community).field("name", name).field("permissions", permissions).field("color", color).field("position", position).finish(),
            Self::RoleUpdate { community, role_id, name, permissions, color } => f.debug_struct("RoleUpdate").field("community", community).field("role_id", role_id).field("name", name).field("permissions", permissions).field("color", color).finish(),
            Self::RoleDelete { community, role_id } => f.debug_struct("RoleDelete").field("community", community).field("role_id", role_id).finish(),
            Self::RoleAssign { community, member_pseudonym, role_id } => f.debug_struct("RoleAssign").field("community", community).field("member_pseudonym", member_pseudonym).field("role_id", role_id).finish(),
            Self::RoleUnassign { community, member_pseudonym, role_id } => f.debug_struct("RoleUnassign").field("community", community).field("member_pseudonym", member_pseudonym).field("role_id", role_id).finish(),
            Self::Kick { community, target_pseudonym } => f.debug_struct("Kick").field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::Ban { community, target_pseudonym, reason } => f.debug_struct("Ban").field("community", community).field("target_pseudonym", target_pseudonym).field("reason", reason).finish(),
            Self::Unban { community, target_pseudonym } => f.debug_struct("Unban").field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::Timeout { community, target_pseudonym, duration_seconds, reason } => f.debug_struct("Timeout").field("community", community).field("target_pseudonym", target_pseudonym).field("duration_seconds", duration_seconds).field("reason", reason).finish(),
            Self::BanList { community } => f.debug_struct("BanList").field("community", community).finish(),
            Self::InviteCreate { community, max_uses, expires_seconds } => f.debug_struct("InviteCreate").field("community", community).field("max_uses", max_uses).field("expires_seconds", expires_seconds).finish(),
            Self::InviteList { community } => f.debug_struct("InviteList").field("community", community).finish(),
            Self::InviteRevoke { community, invite_code } => f.debug_struct("InviteRevoke").field("community", community).field("invite_code", invite_code).finish(),
            Self::ReactionAdd { community, channel, message_id, emoji } => f.debug_struct("ReactionAdd").field("community", community).field("channel", channel).field("message_id", message_id).field("emoji", emoji).finish(),
            Self::ReactionRemove { community, channel, message_id, emoji } => f.debug_struct("ReactionRemove").field("community", community).field("channel", channel).field("message_id", message_id).field("emoji", emoji).finish(),
            Self::PinAdd { community, channel, message_id } => f.debug_struct("PinAdd").field("community", community).field("channel", channel).field("message_id", message_id).finish(),
            Self::PinRemove { community, channel, message_id } => f.debug_struct("PinRemove").field("community", community).field("channel", channel).field("message_id", message_id).finish(),
            Self::EventCreate { community, title, .. } => f.debug_struct("EventCreate").field("community", community).field("title", title).finish(),
            Self::EventUpdate { community, event_id, title, .. } => f.debug_struct("EventUpdate").field("community", community).field("event_id", event_id).field("title", title).finish(),
            Self::EventDelete { community, event_id } => f.debug_struct("EventDelete").field("community", community).field("event_id", event_id).finish(),
            Self::EventRsvp { community, event_id, status } => f.debug_struct("EventRsvp").field("community", community).field("event_id", event_id).field("status", status).finish(),
            Self::EventRemind { community, event_id, title, minutes_until } => f.debug_struct("EventRemind").field("community", community).field("event_id", event_id).field("title", title).field("minutes_until", minutes_until).finish(),
            Self::ThreadCreate { community, channel, title, .. } => f.debug_struct("ThreadCreate").field("community", community).field("channel", channel).field("title", title).finish(),
            Self::ThreadMessage { community, thread_id, mek_generation, .. } => f.debug_struct("ThreadMessage").field("community", community).field("thread_id", thread_id).field("mek_generation", mek_generation).finish(),
            Self::ThreadArchive { community, thread_id, archived } => f.debug_struct("ThreadArchive").field("community", community).field("thread_id", thread_id).field("archived", archived).finish(),
            Self::GameServerAdd { community, game_id, label, address } => f.debug_struct("GameServerAdd").field("community", community).field("game_id", game_id).field("label", label).field("address", address).finish(),
            Self::GameServerRemove { community, server_id } => f.debug_struct("GameServerRemove").field("community", community).field("server_id", server_id).finish(),
            Self::RaidAlert { community, active } => f.debug_struct("RaidAlert").field("community", community).field("active", active).finish(),
            Self::LockdownToggle { community, locked } => f.debug_struct("LockdownToggle").field("community", community).field("locked", locked).finish(),
            Self::KickNotify { community, target_pseudonym } => f.debug_struct("KickNotify").field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::BootstrapRequest { community } => f.debug_struct("BootstrapRequest").field("community", community).finish(),
            Self::BootstrapRespond { community, target_pseudonym, .. } => f.debug_struct("BootstrapRespond").field("community", community).field("target_pseudonym", target_pseudonym).finish(),
            Self::SyncRequest { community, channel_id, since_timestamp } => f.debug_struct("SyncRequest").field("community", community).field("channel_id", channel_id).field("since_timestamp", since_timestamp).finish(),
            Self::SyncRespond { community, target_pseudonym, channel_id, .. } => f.debug_struct("SyncRespond").field("community", community).field("target_pseudonym", target_pseudonym).field("channel_id", channel_id).finish(),
            Self::VoiceJoin { community, channel, muted, deafened } => f.debug_struct("VoiceJoin").field("community", community).field("channel", channel).field("muted", muted).field("deafened", deafened).finish(),
            Self::VoiceLeave => write!(f, "VoiceLeave"),
            Self::VoiceMute { muted } => f.debug_struct("VoiceMute").field("muted", muted).finish(),
            Self::VoiceDeafen { deafened } => f.debug_struct("VoiceDeafen").field("deafened", deafened).finish(),
            Self::BulkTransferStart { transfer_id, total_size, media_type, digest, direction } => f.debug_struct("BulkTransferStart").field("transfer_id", transfer_id).field("total_size", total_size).field("media_type", media_type).field("digest", digest).field("direction", direction).finish(),
            Self::BulkTransferComplete { transfer_id, digest, bytes_transferred } => f.debug_struct("BulkTransferComplete").field("transfer_id", transfer_id).field("digest", digest).field("bytes_transferred", bytes_transferred).finish(),
            Self::BulkTransferCancel { transfer_id, reason } => f.debug_struct("BulkTransferCancel").field("transfer_id", transfer_id).field("reason", reason).finish(),
            Self::BulkTransferStatus { transfer_id } => f.debug_struct("BulkTransferStatus").field("transfer_id", transfer_id).finish(),
            Self::NetworkStatus => write!(f, "NetworkStatus"),
            Self::NetworkPeers => write!(f, "NetworkPeers"),
            Self::AgentRegister { name, agent_type, capabilities } => f.debug_struct("AgentRegister").field("name", name).field("agent_type", agent_type).field("capabilities", capabilities).finish(),
            Self::AgentRevoke { name } => f.debug_struct("AgentRevoke").field("name", name).finish(),
            Self::PolicyReload => write!(f, "PolicyReload"),
            Self::EventResume { last_seen_seq } => f.debug_struct("EventResume").field("last_seen_seq", last_seen_seq).finish(),
        }
    }
}
