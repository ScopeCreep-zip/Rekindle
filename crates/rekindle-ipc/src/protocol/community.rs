//! The community-domain requests (plan E3.1): communities, channels and
//! categories, roles, moderation, invites, game servers and channel keys,
//! carried as `IpcRequest::Community`. Each variant keeps its own name:
//! `IpcRequest::name` answers the leaf, so the intent table, the lanes and
//! the audit log name it as before.

use rekindle_types::dto::onboarding::{OnboardingConfigDto, WelcomeScreenDto};
use rekindle_types::presence::OnboardingAnswer;
use serde::{Deserialize, Serialize};

/// A community-domain request. Every variant is matched in the daemon's
/// community router without a wildcard arm.
#[derive(Clone, Serialize, Deserialize)]
pub enum CommunityRequest {
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

    // ── Profile, pictures and policy (plan E3.1 D4) ──────────────
    /// Our per-community profile: bio, pronouns, theme color, badges and
    /// the avatar and banner hashes. Published with our presence.
    CommunityUpdateProfile {
        community: String,
        bio: Option<String>,
        pronouns: Option<String>,
        theme_color: Option<u32>,
        badges: Vec<String>,
        avatar_ref: Option<String>,
        banner_ref: Option<String>,
    },
    /// Cache `image`, declared to be `kind`, as a community avatar (a WebP
    /// of at most 128 px a side); answered with its BLAKE3 hex hash.
    CommunitySetAvatar {
        community: String,
        image: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },
    /// The same for a banner (at most 600 × 200 px).
    CommunitySetBanner {
        community: String,
        image: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },
    /// A cached avatar or banner by its hash, as a `data:image/webp` URL;
    /// `None` when it is not cached.
    CommunityAvatarGet { community: String, hash: String },
    /// The community's policy text and raid thresholds.
    CommunityPolicyGet { community: String },
    /// Write a new community policy (`MANAGE_COMMUNITY`).
    CommunityPolicySet {
        community: String,
        policy_text: Option<String>,
        max_joins_per_interval: u32,
        join_interval_seconds: u32,
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
    /// Replace the body of our message `message_id` in `channel` (a channel
    /// id; the Host finds its community).
    ChannelMessageEdit {
        channel: String,
        message_id: String,
        body: String,
    },
    /// Delete our message `message_id` in `channel` (a channel id).
    ChannelMessageDelete { channel: String, message_id: String },

    // ── Channel keys (MEK) ─────────────────────────────────────
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

    // ── Audit log and notification default (plan E3.1 F4) ─────────
    /// A page of the community's audit log, newest first: the rows older
    /// than `before_timestamp` (a Lamport time) when given, at most `limit`.
    AuditLog {
        community: String,
        before_timestamp: Option<u64>,
        limit: u32,
    },
    /// The community's default notification level, if one is set.
    NotificationDefaultGet { community: String },
    /// Set the community's default notification level (`all`, `mentions`
    /// or `nothing`).
    NotificationDefaultSet { community: String, level: String },

    // ── Events (plan E3.1 E4) ─────────────────────────────────────
    /// A community's events, by start time, with their attendees.
    EventList { community: String },
    /// One event's attendees.
    EventAttendees { community: String, event_id: String },
    /// Schedule an event; answered with its id.
    EventCreate {
        community: String,
        event: rekindle_types::event::NewEvent,
    },
    /// Change an event's fields.
    EventEdit {
        community: String,
        event_id: String,
        edit: rekindle_types::event::EventEdit,
    },
    /// Cancel an event (kept, marked cancelled).
    EventCancel { community: String, event_id: String },
    /// Delete an event.
    EventDelete { community: String, event_id: String },
    /// Our answer to an event: `going`, `interested` (or `maybe`), `declined`.
    EventRsvp {
        community: String,
        event_id: String,
        status: String,
    },
    // ── Onboarding (plan E3.1 D3) ─────────────────────────────────
    /// A community's onboarding flow (the empty default when none is set).
    OnboardingConfigGet { community: String },
    /// Set a community's onboarding flow (`MANAGE_COMMUNITY`).
    OnboardingConfigSet {
        community: String,
        config: OnboardingConfigDto,
    },
    /// A community's welcome screen (the empty default when none is set).
    WelcomeScreenGet { community: String },
    /// Set a community's welcome screen (`MANAGE_COMMUNITY`).
    WelcomeScreenSet {
        community: String,
        screen: WelcomeScreenDto,
    },
    /// Answer a community's onboarding questions; a gated community needs
    /// `acknowledged_rules`.
    OnboardingSubmit {
        community: String,
        answers: Vec<OnboardingAnswer>,
        acknowledged_rules: bool,
    },
    /// Our onboarding of a community is done.
    OnboardingComplete { community: String },

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
    /// Lift a member's timeout early.
    TimeoutRemove {
        community: String,
        target_pseudonym: String,
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
}
