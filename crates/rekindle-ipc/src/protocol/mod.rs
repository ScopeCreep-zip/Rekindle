//! IPC protocol definitions: Request, Response, and Event enums.
//!
//! These are the typed payloads carried inside `Message<T>`. The bus server
//! routes based on the `Message` envelope; the payload is opaque to routing.
//!
//! Every `IpcRequest` variant maps 1:1 to a `rekindle_transport::operations::*`
//! function or a `QueryEngine` method. There are no catch-all variants — the
//! match in `daemon::dispatch` is exhaustive without a wildcard arm.
//!
//! [RC-16] `Unlock`, `IdentityCreate`, `IdentityExport`, `IdentityImport` and
//! `IdentityRotate` contain secrets — their
//! `Debug` impls redact sensitive fields.

use serde::{Deserialize, Serialize};

use super::message::AgentType;

// ── Subscription filter (re-exported from rekindle-types) ──────────────

pub use rekindle_types::subscription_events::{
    EventCategory, SubscriptionFilter, MAX_FILTERS_PER_CONNECTION,
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
    /// Unlock `identity` (a public key from `IdentityList`) with its
    /// passphrase: open its vault and database, resume its session,
    /// transition to Operational.
    Unlock {
        identity: String,
        passphrase: String,
    },
    /// Lock the daemon (zeroize signing key, transition to Locked).
    Lock,
    /// Log out: announce offline, then lock and deselect the identity.
    Logout,
    /// Query daemon status (always available, any state).
    /// Returns `StatusSnapshot` with compact status, subscription health,
    /// and full diagnostic checks. Renderers decide display depth.
    Status,
    /// Graceful shutdown: drain connections, stop transport, exit.
    /// Returns Ok before the process exits so the client gets confirmation.
    Shutdown,

    // ── Identity ──────────────────────────────────────────────────
    /// Create a new identity and unlock it (plan E1.1): its vault and
    /// database under `passphrase`, then the unlock, whose own-records stage
    /// publishes its DHT records once the network has a route. Without a
    /// name it is `User_` and its key's first eight characters. Answers the
    /// opened identity (`dto::identity::IdentityView`).
    IdentityCreate {
        display_name: Option<String>,
        passphrase: String,
    },
    /// The identities on this device (the registry; no unlock needed).
    IdentityList,
    /// Show the unlocked identity (pubkey, display name, DHT keys).
    IdentityShow,
    /// Export the unlocked identity as an archive (base64): its registry
    /// entry, vault and header, sealed by the vault's passphrase.
    IdentityExport { passphrase: String },
    /// Install an identity from an exported archive (base64); it is
    /// opened with `passphrase` to verify it, and stays locked.
    IdentityImport { archive: String, passphrase: String },
    /// Rotate the unlocked identity's Ed25519 keypair: friends are told,
    /// then the identity is locked and moved to its new key.
    IdentityRotate { passphrase: String },
    /// Delete `identity`. Protected: the host's intent gate asks for its
    /// passphrase (plan D3.3).
    IdentityDelete { identity: String },
    /// Factory reset: delete every identity and Veilid storage. Protected.
    IdentityWipe,

    // ── Friends ───────────────────────────────────────────────────
    /// Send a friend request to a target (mailbox DHT key).
    FriendAdd { target: String, message: String },
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
    CommunityCreate {
        name: String,
        description: String,
        /// Genesis admission mode. `false` (the default) is
        /// `AdmissionMode::Open`; `true` requires an approval before
        /// honest peers count a joiner as a member.
        ///
        /// Chosen at creation and only at creation — the merge honours
        /// `AdmissionPolicy` solely as the genesis entry, so this cannot
        /// be changed later by anyone holding `MANAGE_COMMUNITY`.
        #[serde(default)]
        approval_required: bool,
    },
    /// Join a community via governance key or invite code.
    CommunityJoin { invite: String },
    /// Leave a community.
    CommunityLeave { governance_key: String },
    /// List all joined communities.
    CommunityList,
    /// Get detailed community info (channels, roles, members).
    CommunityInfo { governance_key: String },
    /// Approve a pending member from the waiting room.
    CommunityApprove {
        governance_key: String,
        member_pseudonym: String,
    },
    /// Reject a pending member from the waiting room.
    CommunityReject {
        governance_key: String,
        member_pseudonym: String,
        reason: String,
    },
    /// List pending join requests for a community.
    CommunityPendingMembers { governance_key: String },
    /// Transfer community ownership to a new owner.
    CommunityTransferOwnership {
        governance_key: String,
        new_owner_pseudonym: String,
    },

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
    ChannelDelete {
        community: String,
        channel_id: String,
    },
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
    /// Get channel message history.
    ChannelHistory {
        community: String,
        channel: String,
        limit: u32,
    },

    // ── DMs ───────────────────────────────────────────────────────
    /// Send a direct message.
    DmSend { peer_key: String, body: String },
    /// Send a typing indicator to a peer.
    DmTyping { peer_key: String, typing: bool },
    /// List DM inbox.
    DmInbox { limit: u32 },

    // ── Intents (plan D3.3, ADR 0019) ──────────────────────────────
    /// Commit a held action with the proof its review asked for.
    IntentCommit {
        intent_id: rekindle_types::intent::IntentId,
        proof: rekindle_types::intent::IntentProof,
    },
    /// Drop a held action.
    IntentCancel {
        intent_id: rekindle_types::intent::IntentId,
    },
    /// Take back an action that ran, while its offer lasts (plan E1.1):
    /// the inverse the host stored goes through the gate as a request of
    /// its own.
    Undo {
        id: rekindle_types::intent::IntentId,
    },

    // ── Subscriptions ─────────────────────────────────────────────
    /// Subscribe to events matching filters. `from` is the last cursor this
    /// client processed (plan D3.2): what followed it is replayed, then
    /// `Resumed`, or `InvalidSession` when the host can no longer serve it.
    /// `None` subscribes to live events only.
    Subscribe {
        filters: Vec<SubscriptionFilter>,
        from: Option<rekindle_types::subscription_events::Cursor>,
    },
    /// Unsubscribe from events matching filters.
    Unsubscribe { filters: Vec<SubscriptionFilter> },

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

    // ── Presence and profile (plan E1.1) ──────────────────────────
    /// Set our status. `Offline` is refused: only the host's teardown
    /// writes it; "Appear Offline" is `Invisible`, which peers cannot tell
    /// from it.
    PresenceSet {
        status: rekindle_types::presence::SessionStatus,
    },
    /// Set our status message (`MAX_STATUS_LEN`).
    PresenceSetMessage { message: String },
    /// Our own status, message, game and idle state
    /// (`dto::presence::OwnPresenceView`).
    PresenceGet,
    /// Change the name we show: the identity's database, the registry and
    /// profile subkey 0.
    ProfileSetNickname { display_name: String },
    /// Change our avatar: the image is made a 128 px WebP, saved and
    /// published to profile subkey 3.
    ProfileSetAvatar { image: Vec<u8> },
    /// Our own or a friend's avatar (WebP bytes), from the database.
    AvatarGet { public_key: String },
    /// Set game presence info.
    GamePresenceSet {
        game_name: String,
        game_id: Option<u32>,
        elapsed_seconds: u32,
        server_address: Option<String>,
    },
    /// Clear game presence.
    GamePresenceClear,

    // ── Audit (plan E1.1) ─────────────────────────────────────────
    /// Verify the identity's tamper-evident log end to end
    /// (`dto::audit::AuditVerified`).
    AuditVerify,
    /// The log's entries after `since` (`dto::audit::AuditEntryView`).
    AuditExport { since: u64 },

    // ── Links (plan E1.1) ─────────────────────────────────────────
    /// Open an https link in the system browser: reviewed as
    /// `OpenExternalUrl` (the IP address leaves), answered with the
    /// normalized URL, which the shell opens.
    LinkOpen { url: String },

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
    Kick {
        community: String,
        target_pseudonym: String,
    },
    /// Ban a member (persist to governance + gossip broadcast).
    Ban {
        community: String,
        target_pseudonym: String,
        reason: Option<String>,
    },
    /// Unban a member.
    Unban {
        community: String,
        target_pseudonym: String,
    },
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
    InviteRevoke {
        community: String,
        invite_code: String,
    },

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

mod capability;
mod debug;
mod lane;
mod response;

pub use capability::AgentCapability;
pub use lane::Lane;

pub use response::{BusPayload, IpcResponse};
