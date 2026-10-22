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
/// Variant naming: `{Domain}{Verb}` — e.g., `ChannelCreate`, `FriendRemove`.
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
    /// Our safety number with `peer` and whether we verified their key
    /// (`dto::friends::SafetyNumberView`, plan E2.3).
    IdentitySafetyNumber { peer: String },
    /// Mark `peer`'s identity key verified, after comparing the safety
    /// number out of band; acknowledges an unacknowledged key change.
    IdentityMarkVerified { peer: String },
    /// Withdraw `peer`'s verification.
    IdentityClearVerification { peer: String },

    // ── Friends (plan E2.1) ───────────────────────────────────────
    /// Send a friend request to `public_key` (they become a pending friend).
    /// Their name is the Host's (plan audit U-7), never the caller's.
    /// Send a friend request to the identity a friend code names
    /// (`rekindle://add/<key>/<profile>/<token>`, plan E2.5 D17).
    FriendAddFromCode { code: String, message: String },
    /// Accept the pending request from `public_key` (named by the request).
    FriendAccept { public_key: String },
    /// Reject the pending request from `public_key`.
    FriendReject { public_key: String },
    /// Remove a friend.
    FriendRemove { public_key: String },
    /// The buddy list (`dto::friends::Friend`).
    FriendList,
    /// The requests waiting for our answer.
    FriendRequests,
    /// Cancel the request we sent `public_key`.
    FriendCancelRequest { public_key: String },
    /// Retry the first contact with `public_key` that failed (plan E2.4,
    /// B7): our accept, or our reset accept when that was the one pending,
    /// from a fresh slot of their prekey record.
    FriendRetryHandshake { public_key: String },
    /// Add the friend an invite link names (reviewed with their key's
    /// fingerprint).
    FriendAddFromInvite { invite: String },
    /// Make a tracked friend invite link.
    FriendInviteCreate,
    /// Our open invites.
    FriendInviteList,
    /// Cancel an open invite.
    FriendInviteCancel { invite_id: String },
    /// Our friend code, its parts and its QR (`dto::friends::FriendCodeView`,
    /// plan E2.5 D17).
    FriendCode,
    /// Mint a new friend-request token: codes carrying the old one stop
    /// working and friends are untouched (`dto::friends::FriendCodeView`).
    FriendTokenRotate,
    /// Block `public_key` (our profile moves to a new key).
    FriendBlock { public_key: String },
    /// Unblock `public_key`.
    FriendUnblock { public_key: String },
    /// Everyone we blocked.
    FriendBlockedList,
    /// Set (or clear) our nickname for a friend.
    FriendSetNickname {
        public_key: String,
        nickname: Option<String>,
    },
    /// Create a friend group.
    FriendGroupCreate { name: String },
    /// Rename a friend group.
    FriendGroupRename { group_id: i64, name: String },
    /// Move a friend into a group, or out of any with `None`.
    FriendGroupMove {
        public_key: String,
        group_id: Option<i64>,
    },
    /// Delete a friend group; its friends fall back to no group.
    FriendGroupDelete { group_id: i64 },
    /// Reset our secure session with a friend (safety-number checked).
    SessionResetStart { public_key: String },
    /// Re-initiate our secure session with a friend whose retry requests
    /// are exhausted (plan E2.5 last stage, ruling Q1): the backend holds
    /// it for the user's review, and commit runs the same reset.
    SessionReinitiate { public_key: String },
    /// Accept the session reset a friend asked for.
    SessionResetAccept { public_key: String },
    /// Decline the session reset a friend asked for.
    SessionResetDecline { public_key: String },
    /// Accept the identity rotation a friend (by their current key) sent:
    /// they move to the new key (owner ruling R1).
    IdentityRotationAccept { public_key: String },
    /// Decline a friend's identity rotation: it is dropped, nothing moves.
    IdentityRotationDecline { public_key: String },

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
    /// Every joined community as its window renders it (channels,
    /// categories, roles, our standing).
    CommunityViews,
    /// A community's member list.
    CommunityMembers { governance_key: String },
    /// Get detailed community info (channels, roles, members).
    CommunityInfo { governance_key: String },
    /// Edit a community's name, description, icon or banner; `None` keeps
    /// the merged value.
    CommunityUpdateInfo {
        governance_key: String,
        name: Option<String>,
        description: Option<String>,
        icon_hash: Option<String>,
        banner_hash: Option<String>,
    },
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
        /// The voice channel a text-in-voice channel belongs to.
        parent_voice_channel: Option<String>,
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
    /// Rename a channel.
    ChannelRename {
        community: String,
        channel_id: String,
        name: String,
    },
    /// Move a channel into a category, or out of every category (`None`).
    ChannelMove {
        community: String,
        channel_id: String,
        category: Option<String>,
    },
    /// Order a community's channels: each gets its index as its position.
    ChannelReorder {
        community: String,
        channel_ids: Vec<String>,
    },
    /// Set a channel's topic.
    ChannelSetTopic {
        community: String,
        channel_id: String,
        topic: String,
    },
    /// Set a forum channel's tags.
    ChannelSetForumTags {
        community: String,
        channel_id: String,
        forum_tags: Vec<String>,
    },
    /// Set a channel's permission overwrite for a role or member.
    ChannelOverwriteSet {
        community: String,
        channel_id: String,
        target_type: String,
        target_id: String,
        allow: u64,
        deny: u64,
    },
    /// Clear a channel's permission overwrite for a role or member.
    ChannelOverwriteDelete {
        community: String,
        channel_id: String,
        target_type: String,
        target_id: String,
    },
    /// Create a channel category.
    CategoryCreate { community: String, name: String },
    /// Delete a channel category; its channels leave it.
    CategoryDelete {
        community: String,
        category_id: String,
    },
    /// Rename a channel category.
    CategoryRename {
        community: String,
        category_id: String,
        name: String,
    },
    /// Order a community's categories: each gets its index as its position.
    CategoryReorder {
        community: String,
        category_ids: Vec<String>,
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
    DmSend {
        peer_key: String,
        body: String,
        /// The parent message's client id (a UUID) when this is a reply.
        reply_to: Option<String>,
    },
    /// The draft to a peer changed: `typing` is whether it holds text. The
    /// Host decides what goes out (the start, its refresh, the idle stop;
    /// plan E2.3).
    DmTyping { peer_key: String, typing: bool },
    /// The latest message of each direct conversation.
    DmInbox { limit: u32 },
    /// The newest `limit` messages with `peer_key` before `before` (ms).
    DmHistory {
        peer_key: String,
        limit: u32,
        before: Option<i64>,
    },
    /// Mark everything from `peer_key` read; `rendered_through` is the
    /// client id of the newest of their messages the focused window
    /// rendered, for our read marker (plan E2.5).
    DmMarkRead {
        peer_key: String,
        rendered_through: Option<String>,
    },
    /// Retry an outbox row by its client id (plan E2.5): queued again
    /// under the same id, and no new row is stored.
    OutboxRetry { client_id: String },
    /// Discard a failed outbox row: it settles failed and leaves its
    /// stream.
    OutboxDiscard { client_id: String },

    // ── Group DMs (plan E2.6) ──────────────────────────────────────
    /// Create a group named `name` of us and `members` (2-7 friends'
    /// keys); the answer is its id.
    GroupDmCreate { name: String, members: Vec<String> },
    /// Accept the pending invite to `group_id` (32 hex).
    GroupDmAccept { group_id: String },
    /// Decline the pending invite to `group_id`.
    GroupDmDecline { group_id: String },
    /// Leave `group_id`; every other member rotates its key.
    GroupDmLeave { group_id: String },
    /// Send `body` to `group_id`; the answer is the message's client id
    /// once its row and local echo are committed (`queued`, then
    /// `GroupDmEvent::SendStatus`).
    GroupDmSend { group_id: String, body: String },
    /// The draft to `group_id` changed: `typing` is whether it holds text.
    /// The Host decides what goes out, as for `DmTyping`.
    GroupDmTyping { group_id: String, typing: bool },
    /// Every group we are in and every invite we have not answered.
    GroupDmList,
    /// The newest `limit` messages of `group_id` before `before` (ms).
    GroupDmMessages {
        group_id: String,
        limit: u32,
        before: Option<i64>,
    },

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
    /// This device's prekey record and its upkeep (plan E2.4):
    /// `dto::identity::PrekeyStatus`.
    PrekeyStatus,
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
    /// Turn DM read receipts on or off (plan E2.5, r10): the identity's
    /// `Preferences.read_receipts_enabled`; the preferences come back.
    SetReadReceiptsEnabled { enabled: bool },
    /// Change our avatar: the image, of the kind its sender declares (the
    /// file picker's MIME type, the file's extension), is decoded by
    /// `rekindle-image` as that kind only, made a 128 px WebP, saved and
    /// published to profile subkey 3.
    ProfileSetAvatar {
        image: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },
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

    // ── Game servers (plan E3.1 H1) ───────────────────────────────
    /// A community's game servers, newest first.
    GameServerList { community: String },
    /// Add a game server to a community; answered with its id.
    GameServerAdd {
        community: String,
        game_id: String,
        label: String,
        address: String,
    },
    /// Remove a community's game server.
    GameServerRemove {
        community: String,
        server_id: String,
    },

    // ── Roles ─────────────────────────────────────────────────────
    /// List all roles in a community.
    RoleList { community: String },
    /// Create a new role.
    RoleCreate {
        community: String,
        name: String,
        permissions: u64,
        color: u32,
        /// Members with it are listed in their own sidebar group.
        hoist: bool,
        /// `@role` reaches everyone holding it.
        mentionable: bool,
        /// Members may take it themselves.
        self_assignable: bool,
        /// At most one role of a group per member (architecture §19.4).
        exclusion_group: Option<String>,
    },
    /// Update a role's properties.
    RoleUpdate {
        community: String,
        role_id: u32,
        name: Option<String>,
        permissions: Option<u64>,
        color: Option<u32>,
        position: Option<i32>,
        hoist: Option<bool>,
        mentionable: Option<bool>,
        self_assignable: Option<bool>,
        exclusion_group: rekindle_types::governance::ExclusionGroupEdit,
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
    /// Take a self-assignable role ourselves.
    RoleSelfAssign { community: String, role_id: u32 },
    /// Drop a self-assignable role we hold.
    RoleSelfUnassign { community: String, role_id: u32 },

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
