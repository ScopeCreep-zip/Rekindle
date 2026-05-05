use serde::{Deserialize, Serialize};

/// Events streamed from Rust to the frontend for community operations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum CommunityEvent {
    /// Architecture §18.4 — eager-fetched expression bytes have landed in
    /// the local cache. Frontend should re-query `list_expressions` so
    /// the picker swaps the `:emojiname:` placeholder for the resolved
    /// inline_data_base64.
    #[serde(rename_all = "camelCase")]
    ExpressionAssetReady {
        community_id: String,
        expression_id: String,
    },
    #[serde(rename_all = "camelCase")]
    MemberJoined {
        community_id: String,
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<u32>,
    },
    #[serde(rename_all = "camelCase")]
    MemberRemoved {
        community_id: String,
        pseudonym_key: String,
    },
    /// Architecture §20.6 — peer-side raid detector. Emitted by every
    /// peer that observes the join rate exceeding
    /// `CommunityPolicy.max_joins_per_interval` within
    /// `CommunityPolicy.join_interval_seconds`. Moderators in that
    /// client get a banner / toast and may pause invites or ban floods
    /// (the spec lists those as the moderator-side responses).
    #[serde(rename_all = "camelCase")]
    RaidDetected {
        community_id: String,
        joins_in_window: u32,
        max_joins_per_interval: u32,
        join_interval_seconds: u32,
    },
    #[serde(rename_all = "camelCase")]
    MekRotated {
        community_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<String>,
        new_generation: u64,
    },
    /// We were kicked from a community (our pseudonym was removed by an admin).
    #[serde(rename_all = "camelCase")]
    Kicked { community_id: String },
    /// A member's assigned roles were changed.
    #[serde(rename_all = "camelCase")]
    MemberRolesChanged {
        community_id: String,
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },
    /// A member was timed out or their timeout was removed.
    #[serde(rename_all = "camelCase")]
    MemberTimedOut {
        community_id: String,
        pseudonym_key: String,
        timeout_until: Option<u64>,
    },
    /// Channel permission overwrites were changed (server-side enforcement).
    #[serde(rename_all = "camelCase")]
    ChannelOverwriteChanged {
        community_id: String,
        channel_id: String,
    },
    /// A message was edited in a channel.
    #[serde(rename_all = "camelCase")]
    MessageEdited {
        community_id: String,
        channel_id: String,
        message_id: String,
        new_body: String,
        edited_at: u64,
    },
    /// A message was deleted from a channel.
    #[serde(rename_all = "camelCase")]
    MessageDeleted {
        community_id: String,
        channel_id: String,
        message_id: String,
    },
    /// A reaction was added to a message.
    #[serde(rename_all = "camelCase")]
    ReactionAdded {
        community_id: String,
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    /// A reaction was removed from a message.
    #[serde(rename_all = "camelCase")]
    ReactionRemoved {
        community_id: String,
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    /// A message was pinned.
    #[serde(rename_all = "camelCase")]
    MessagePinned {
        community_id: String,
        channel_id: String,
        message_id: String,
        pinned_by: String,
    },
    /// A message was unpinned.
    #[serde(rename_all = "camelCase")]
    MessageUnpinned {
        community_id: String,
        channel_id: String,
        message_id: String,
    },
    /// A queued channel message was eventually delivered after retry.
    #[serde(rename_all = "camelCase")]
    ChannelMessageDelivered {
        community_id: String,
        channel_id: String,
        message_id: String,
    },
    /// A queued channel message permanently failed after all retry attempts.
    #[serde(rename_all = "camelCase")]
    ChannelMessageDeliveryFailed {
        community_id: String,
        channel_id: String,
        message_id: String,
    },
    /// A member started typing in a channel.
    #[serde(rename_all = "camelCase")]
    ChannelTyping {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// Architecture §10.9: a member triggered a soundboard sound in a
    /// voice channel. The frontend looks up the expression in the local
    /// cache and plays the audio.
    #[serde(rename_all = "camelCase")]
    SoundboardPlay {
        community_id: String,
        channel_id: String,
        expression_id: String,
        actor_pseudonym: String,
    },
    /// Architecture §10.6: a complete video / screen-share frame has
    /// been reassembled and MEK-decrypted. The frontend hands the bytes
    /// to a WebCodecs `VideoDecoder` for rendering.
    #[serde(rename_all = "camelCase")]
    VideoFrame {
        community_id: String,
        sender_pseudonym: String,
        stream_id: String,
        frame_seq: u32,
        keyframe: bool,
        timestamp: u32,
        /// Base64-encoded decrypted VP9 frame.
        payload_b64: String,
    },
    /// Architecture §10.6 receiver acknowledgement — surfaces upstream
    /// kbps so the encoder can adapt VP9 bitrate.
    #[serde(rename_all = "camelCase")]
    VideoFrameAck {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: String,
        last_frame_seq: u32,
        kbps: u32,
        loss_q8: u8,
    },
    /// Architecture §10.6 — receiver requests an I-frame.
    #[serde(rename_all = "camelCase")]
    VideoKeyframeRequest {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: String,
    },
    /// Architecture §10.6 — receiver advertises measured bandwidth
    /// outside of a frame round-trip.
    #[serde(rename_all = "camelCase")]
    VideoBandwidthEstimate {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        kbps: u32,
        window_secs: u8,
        loss_q8: u8,
    },
    /// Architecture §10.6 line 4084 — peer's decode capabilities.
    #[serde(rename_all = "camelCase")]
    VideoMediaCapabilities {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        max_pixel_count: u32,
        max_fps: u8,
        codecs: Vec<String>,
    },
    /// Architecture §10.6 + Phase 6 Week 22 — the active video relay
    /// for a `(channel_id, stream_id)` changed. Frontend should
    /// re-attach its decoder to the new relay's stream and discard any
    /// partially-buffered frames from the old one.
    #[serde(rename_all = "camelCase")]
    VideoTopologyChange {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: String,
        relay_host_pseudonym: Option<String>,
        reason: String,
        lamport: u64,
    },
    /// Architecture §28.8 — sender pre-fetched OpenGraph metadata for
    /// a URL embedded in `message_id`.
    #[serde(rename_all = "camelCase")]
    LinkPreviewReceived {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        message_id: String,
        url: String,
        title: Option<String>,
        description: Option<String>,
        image_url: Option<String>,
        site_name: Option<String>,
        fetched_at: u64,
    },
    /// A member's presence status changed.
    #[serde(rename_all = "camelCase")]
    MemberPresenceChanged {
        community_id: String,
        pseudonym_key: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_seconds: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server_address: Option<String>,
    },
    /// A community event was created.
    #[serde(rename_all = "camelCase")]
    EventCreated {
        community_id: String,
        event: EventInfoDto,
    },
    /// A community event was updated.
    #[serde(rename_all = "camelCase")]
    EventUpdated {
        community_id: String,
        event: EventInfoDto,
    },
    /// A community event was deleted.
    #[serde(rename_all = "camelCase")]
    EventDeleted {
        community_id: String,
        event_id: String,
    },
    /// Someone RSVPed to a community event.
    #[serde(rename_all = "camelCase")]
    EventRsvpChanged {
        community_id: String,
        event_id: String,
        pseudonym_key: String,
        status: String,
    },
    /// A thread was created in a channel.
    #[serde(rename_all = "camelCase")]
    ThreadCreated {
        community_id: String,
        thread: ThreadInfoDto,
    },
    /// A new message in a thread.
    #[serde(rename_all = "camelCase")]
    ThreadMessageReceived {
        community_id: String,
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        body: String,
        timestamp: u64,
        reply_to_id: Option<String>,
    },
    /// A thread was archived or unarchived.
    #[serde(rename_all = "camelCase")]
    ThreadArchived {
        community_id: String,
        thread_id: String,
        archived: bool,
    },
    /// A game server was added to the community's favorites.
    #[serde(rename_all = "camelCase")]
    GameServerAdded {
        community_id: String,
        server: GameServerInfoDto,
    },
    /// A game server was removed from the community's favorites.
    #[serde(rename_all = "camelCase")]
    GameServerRemoved {
        community_id: String,
        server_id: String,
    },
    /// An event is starting soon — reminder broadcast.
    #[serde(rename_all = "camelCase")]
    EventReminder {
        community_id: String,
        event_id: String,
        title: String,
        minutes_until_start: u32,
    },
    /// Local AutoMod alert for moderators on this client.
    #[serde(rename_all = "camelCase")]
    AutoModAlert {
        community_id: String,
        channel_id: String,
        message_id: String,
        rule_name: String,
    },
    /// The member list for a community was refreshed (e.g., after DHT update).
    /// Frontend should re-fetch members via `getCommunityMembers`.
    #[serde(rename_all = "camelCase")]
    MembersRefreshed { community_id: String },
    /// System message (join/leave/kick/ban events posted inline in chat).
    #[serde(rename_all = "camelCase")]
    SystemMessage {
        community_id: String,
        body: String,
        timestamp: u64,
    },
    /// Raid alert broadcast — owners/admins should take action.
    #[serde(rename_all = "camelCase")]
    RaidAlert { community_id: String, active: bool },
    /// Channel lockdown broadcast — non-admins should restrict sending.
    #[serde(rename_all = "camelCase")]
    ChannelLockdown { community_id: String, locked: bool },
    /// A member completed onboarding — their roles were assigned.
    #[serde(rename_all = "camelCase")]
    OnboardingComplete {
        community_id: String,
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },
    /// Join request was rejected by a peer or admin.
    #[serde(rename_all = "camelCase")]
    JoinRejected {
        community_id: String,
        reason: String,
    },
    /// Join accepted by a peer — MEK and community data received.
    #[serde(rename_all = "camelCase")]
    JoinAccepted { community_id: String },
    /// Sync response received — channel messages were merged from an archiver.
    /// Frontend should refresh the channel's message list.
    #[serde(rename_all = "camelCase")]
    SyncComplete {
        community_id: String,
        channel_id: String,
        message_count: usize,
    },
    /// CRDT governance state was rebuilt from DHT. Frontend should re-fetch
    /// community details (channels, roles, members, permissions).
    #[serde(rename_all = "camelCase")]
    GovernanceUpdated { community_id: String },
    /// A member joined a voice channel.
    #[serde(rename_all = "camelCase")]
    VoiceJoin {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
        route_blob: Vec<u8>,
    },
    /// A member left a voice channel.
    #[serde(rename_all = "camelCase")]
    VoiceLeave {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// Voice channel mode switched (mesh ↔ MCU).
    #[serde(rename_all = "camelCase")]
    VoiceModeSwitch {
        community_id: String,
        channel_id: String,
        mode: String,
        host_pseudonym: Option<String>,
    },
    /// Stage channel speaker/topic update.
    #[serde(rename_all = "camelCase")]
    StageUpdate {
        community_id: String,
        channel_id: String,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator_pseudonym: String,
    },
    /// Local moderator-facing notification for a speak request.
    #[serde(rename_all = "camelCase")]
    SpeakRequest {
        community_id: String,
        channel_id: String,
        requester_pseudonym: String,
    },
    /// Response to our stage speak request.
    #[serde(rename_all = "camelCase")]
    SpeakResponse {
        community_id: String,
        channel_id: String,
        requester_pseudonym: String,
        granted: bool,
        moderator_pseudonym: String,
    },
    /// Lost Cargo: a download finished — `local_path` is the on-disk file.
    /// Frontend updates the message bubble's "Download" button to "Open".
    #[serde(rename_all = "camelCase")]
    AttachmentDownloaded {
        community_id: String,
        channel_id: String,
        attachment_id: String,
        local_path: String,
    },
    /// Architecture §6 — emitted after any role mutation
    /// (RoleDefinition, RoleArchived, RolePermissionUpdate). The full
    /// merged role list is included so the receiver can replace its
    /// `roles` array atomically without a refetch.
    #[serde(rename_all = "camelCase")]
    RolesChanged {
        community_id: String,
        roles: Vec<RoleDto>,
    },
    /// Architecture §6 — emitted after any channel/category mutation
    /// (ChannelCreated, ChannelArchived, ChannelUpdated, CategoryCreated,
    /// CategoryArchived, CategoryUpdated). Carries snapshots of the
    /// merged channel + category lists so receivers can re-render the
    /// channel tree in one atomic update.
    #[serde(rename_all = "camelCase")]
    ChannelsUpdated {
        community_id: String,
        channels: Vec<ChannelsUpdatedChannelDto>,
        categories: Vec<ChannelsUpdatedCategoryDto>,
    },
    /// Architecture §32 Phase 5 W15 — emitted after `update_community_info`
    /// persists. Carries the new name/description/icon/banner so the
    /// buddy-list and community window can refresh without a full
    /// `getCommunityDetails` round-trip.
    #[serde(rename_all = "camelCase")]
    CommunityUpdated {
        community_id: String,
        name: Option<String>,
        description: Option<String>,
        icon_hash: Option<String>,
        banner_hash: Option<String>,
    },
    /// Architecture §16 — emitted after `create_community_invite`
    /// successfully persists. Carries the new invite metadata (the
    /// raw code is only returned synchronously to the creator).
    #[serde(rename_all = "camelCase")]
    InviteCreated {
        community_id: String,
        code_hash: String,
        created_by: String,
        max_uses: Option<u32>,
        uses: u32,
        expires_at: Option<u64>,
        created_at: u64,
    },
    /// Architecture §16 — emitted when a peer's `MemberJoinRequest`
    /// validates an invite. Increments the InvitesTab "uses" counter
    /// without a refetch.
    #[serde(rename_all = "camelCase")]
    InviteUsed {
        community_id: String,
        code_hash: String,
        new_use_count: u32,
    },
    /// Architecture §16 — emitted after `revoke_community_invite`.
    #[serde(rename_all = "camelCase")]
    InviteRevoked {
        community_id: String,
        code_hash: String,
    },
    /// Architecture §15 — presence poll observed a previously-unknown
    /// subkey reporting in. Emitted from
    /// `services/community/presence/poll.rs::persist_discovered_registry_members`
    /// once per newly-discovered pseudonym so the member list shows
    /// the joiner without a registry refetch.
    #[serde(rename_all = "camelCase")]
    MemberDiscovered {
        community_id: String,
        pseudonym_key: String,
        display_name: String,
        subkey_index: u32,
    },
}

/// Channel snapshot variant emitted in `ChannelsUpdated`. Mirrors
/// the TS type at `src/ipc/channels.ts::channelsUpdated` exactly so
/// the frontend handler can consume without an extra transformer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsUpdatedChannelDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowmode_seconds: Option<u32>,
}

/// Category snapshot variant emitted in `ChannelsUpdated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsUpdatedCategoryDto {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// Event info DTO for frontend consumption (architecture §21).
///
/// Type alias for the canonical wire/in-memory shape defined in
/// `rekindle-types::event::EventInfo`. The DTO name is preserved so
/// existing call sites and IPC consumers don't need to be renamed
/// during the typed-envelope migration (plan:
/// `.claude/plans/community-envelope-capnp-migration.md`).
pub type EventInfoDto = rekindle_types::event::EventInfo;

/// RSVP entry DTO — alias for `rekindle-types::event::EventRsvp`.
pub type EventRsvpInfoDto = rekindle_types::event::EventRsvp;

/// Role DTO for frontend consumption (mirrors protocol's `RoleDto`).
///
/// `permissions` is serialized as a string to avoid JavaScript `Number` precision
/// loss — `u64` values above `2^53 - 1` lose low bits when parsed as JSON numbers,
/// which silently strips the ADMINISTRATOR flag (bit 3) from the Owner role.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDto {
    pub id: u32,
    pub name: String,
    pub color: u32,
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_u64_as_string",
        deserialize_with = "crate::serde_helpers::deserialize_u64_from_string_or_number"
    )]
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    #[serde(default)]
    pub self_assignable: bool,
    /// Architecture §19.4 — when set, only one role per group may be active per member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_group: Option<String>,
}

impl From<&rekindle_protocol::messaging::RoleDto> for RoleDto {
    fn from(dto: &rekindle_protocol::messaging::RoleDto) -> Self {
        Self {
            id: dto.id,
            name: dto.name.clone(),
            color: dto.color,
            permissions: dto.permissions,
            position: dto.position,
            hoist: dto.hoist,
            mentionable: dto.mentionable,
            self_assignable: dto.self_assignable,
            exclusion_group: None,
        }
    }
}

/// Thread info DTO for frontend consumption.
/// Thread info DTO — alias for `rekindle-types::thread::ThreadInfo`.
pub type ThreadInfoDto = rekindle_types::thread::ThreadInfo;

/// Game server info DTO — alias for
/// `rekindle-types::game_server::GameServerInfo`.
pub type GameServerInfoDto = rekindle_types::game_server::GameServerInfo;

impl From<&crate::state::RoleDefinition> for RoleDto {
    fn from(def: &crate::state::RoleDefinition) -> Self {
        Self {
            id: def.id,
            name: def.name.clone(),
            color: def.color,
            permissions: def.permissions,
            position: def.position,
            hoist: def.hoist,
            mentionable: def.mentionable,
            self_assignable: def.self_assignable,
            exclusion_group: def.exclusion_group.clone(),
        }
    }
}

impl From<&rekindle_protocol::dht::community::types::RoleEntryV2> for RoleDto {
    fn from(r: &rekindle_protocol::dht::community::types::RoleEntryV2) -> Self {
        Self {
            id: r.id,
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
            self_assignable: r.self_assignable,
            exclusion_group: None,
        }
    }
}
