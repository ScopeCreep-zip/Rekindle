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
    /// Set a channel's slowmode: the seconds a member waits between posts
    /// (0 turns it off).
    ChannelSetSlowmode {
        community: String,
        channel_id: String,
        seconds: u32,
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
        /// The message id of the message this one replies to.
        reply_to: Option<String>,
    },
    /// Replace the body of our message `message_id` in `channel` (a channel
    /// id; the Host finds its community).
    ChannelMessageEdit {
        channel: String,
        message_id: String,
        body: String,
    },
    /// Post `body` to `channel` (a channel id; the Host finds its community).
    ChannelMessageSend {
        channel: String,
        body: String,
        /// The message id of the message this one replies to.
        reply_to: Option<String>,
    },
    /// Delete our message `message_id` in `channel` (a channel id).
    ChannelMessageDelete { channel: String, message_id: String },
    /// Forward the stored message `source_message` of `source_channel` (in
    /// `source_community`) into `dest_channel` of `dest_community`.
    ChannelMessageForward {
        source_community: String,
        source_channel: String,
        source_message: String,
        dest_community: String,
        dest_channel: String,
    },
    /// Delete another member's message `message_id` in `channel` of
    /// `community` (`MANAGE_MESSAGES`).
    ChannelMessageAdminDelete {
        community: String,
        channel: String,
        message_id: String,
        reason: Option<String>,
    },
    /// Delete up to 100 messages in `channel` of `community`
    /// (`MANAGE_MESSAGES`): how many were deleted.
    ChannelMessageBulkDelete {
        community: String,
        channel: String,
        message_ids: Vec<String>,
        reason: Option<String>,
    },
    /// Tell the community's mesh we are typing in `channel`.
    ChannelTyping { community: String, channel: String },
    /// The channel we are in (`kind` "voice" or "text"), or none, which our
    /// presence carries.
    ChannelSetActive {
        community: String,
        channel: Option<String>,
        kind: String,
    },
    /// The latest `limit` messages of `channel` (a channel id; the Host
    /// finds its community): our stored rows merged with the channel
    /// record's (plan E3.1 E2c).
    ChannelMessages { channel: String, limit: u32 },
    /// Up to `limit` messages of `channel` in `community` older than
    /// `before_timestamp`, merged the same way (a page further back).
    ChannelMessagesBefore {
        community: String,
        channel: String,
        before_timestamp: u64,
        limit: u32,
    },

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

    // ── Unread (plan E3.1 G1, D25) ────────────────────────────────
    /// Advance our read marker on `channel` to the message `up_to` (its
    /// lamport); a marker never moves back.
    ChannelMarkRead {
        community: String,
        channel: String,
        up_to: String,
    },
    /// Each channel's unread messages and mentions, counted after our read
    /// markers.
    UnreadCounts { community: String },

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
    /// Block a member in this community, by the pseudonym they post under
    /// there (D26): our ingest gate drops their posts. Only we see it.
    MemberBlock {
        community: String,
        target_pseudonym: String,
    },
    /// Lift a community block.
    MemberUnblock {
        community: String,
        target_pseudonym: String,
    },
    /// The pseudonyms we block in a community, newest first.
    MemberBlockList { community: String },
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
    /// The community's enabled automod rules.
    AutomodRules { community: String },
    /// Create a rule (no `rule_id`) or replace one; answered with its id.
    AutomodRuleSet {
        community: String,
        rule_id: Option<String>,
        name: String,
        enabled: bool,
        keywords: Vec<String>,
        regex_patterns: Vec<String>,
        action: rekindle_types::governance::AutoModAction,
    },
    /// Turn a rule off for good.
    AutomodRuleDelete { community: String, rule_id: String },
    /// The community's moderator queue (`MANAGE_MESSAGES`).
    ModReportList { community: String },

    // ── Pins and search (E3.1 E3b) ────────────── ───────────────────
    /// A channel's pinned messages.
    ChannelPins {
        community: String,
        channel_id: String,
    },
    /// Pin a channel message (`MANAGE_MESSAGES`).
    MessagePin {
        community: String,
        channel_id: String,
        message_id: String,
    },
    /// Unpin a channel message (`MANAGE_MESSAGES`).
    MessageUnpin {
        community: String,
        channel_id: String,
        message_id: String,
    },
    /// Pin or unpin an attachment, exempting it from cache eviction
    /// (`MANAGE_COMMUNITY`).
    AttachmentPin {
        community: String,
        attachment_id: String,
        pinned: bool,
    },
    /// Full-text search over the stored messages.
    MessageSearch {
        request: rekindle_types::search::MessageSearch,
    },

    // ── Reactions, threads and polls (plan E3.1 E3) ──────────────
    /// React to a channel message with an expression.
    ReactionAdd {
        community: String,
        channel_id: String,
        message_id: String,
        emoji: String,
    },
    /// Take our reaction off a channel message.
    ReactionRemove {
        community: String,
        channel_id: String,
        message_id: String,
        emoji: String,
    },
    /// Open a thread off a channel message; answered with its id.
    ThreadCreate {
        community: String,
        channel_id: String,
        name: String,
        starter_message_id: String,
        forum_tag: Option<String>,
        auto_archive_seconds: Option<u64>,
    },
    /// Archive a thread.
    ThreadArchive {
        community: String,
        thread_id: String,
    },
    /// Reopen an archived thread.
    ThreadUnarchive {
        community: String,
        thread_id: String,
    },
    /// A channel's threads, the most recently active first.
    ThreadList {
        community: String,
        channel_id: String,
    },
    /// A channel's threads that are not archived.
    ThreadListActive {
        community: String,
        channel_id: String,
    },
    /// A page of a thread's messages, oldest first, before
    /// `before_timestamp` (seconds) when given.
    ThreadMessages {
        community: String,
        thread_id: String,
        limit: u32,
        before_timestamp: Option<u64>,
    },
    /// Post `body` to a thread.
    ThreadSend {
        community: String,
        thread_id: String,
        body: String,
    },
    /// Attach a poll to our message; answered with the poll's id.
    PollCreate {
        community: String,
        channel_id: String,
        message_id: String,
        question: String,
        answers: Vec<String>,
        multi_select: bool,
        duration_seconds: Option<u64>,
    },
    /// Vote in a poll (our latest vote counts).
    PollVote {
        community: String,
        channel_id: String,
        poll_id: String,
        selected_answers: Vec<u8>,
    },
    /// Close a poll (its author, or a member who manages messages).
    PollClose {
        community: String,
        channel_id: String,
        poll_id: String,
    },
    /// A poll's vote count per answer.
    PollResults {
        community: String,
        channel_id: String,
        poll_id: String,
    },
    // ── Expressions (E3.1 E3c) ─────────────────────────────────────
    /// Upload a custom emoji (`MANAGE_EXPRESSIONS`): `kind` is what the
    /// picker declared, and the picture is decoded as that kind only.
    EmojiUpload {
        community: String,
        name: String,
        bytes: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },
    /// Upload a custom sticker (`MANAGE_EXPRESSIONS`).
    StickerUpload {
        community: String,
        name: String,
        bytes: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
        tags: Vec<String>,
    },
    /// Delete a custom expression (`MANAGE_EXPRESSIONS`).
    ExpressionDelete {
        community: String,
        expression_id: String,
    },
    /// The community's expressions, with the cached pictures inline.
    Expressions { community: String },

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
