use rekindle_types::video::{Codec, ScalabilityMode};
use rekindle_video::SessionVideoConfig;
use serde::Serialize;

mod dto;
pub use dto::{
    ChannelsUpdatedCategoryDto, ChannelsUpdatedChannelDto, EventInfoDto, EventRsvpInfoDto,
    GameServerInfoDto, RoleDto, ThreadInfoDto,
};

/// One voice-roster participant as shipped to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRosterParticipantEvent {
    pub pseudonym_key: String,
    pub display_name: Option<String>,
}

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
    /// Architecture §10.6 line 4084 — peer's capability advertisement,
    /// direction-split into encode + decode codec lists (WebView
    /// engines are asymmetric; see `rekindle_video::MediaCapabilities`).
    /// Typed codec / scalability-mode lists let the frontend store
    /// reconcile against the typed `Codec` / `ScalabilityMode` enums
    /// without parsing strings.
    #[serde(rename_all = "camelCase")]
    VideoMediaCapabilities {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        max_pixel_count: u32,
        max_fps: u8,
        encode_codecs: Vec<Codec>,
        decode_codecs: Vec<Codec>,
        supports_optimize_for_latency: bool,
        supported_scalability_modes: Vec<ScalabilityMode>,
    },
    /// Architecture §10.6 — backend-negotiated per-call video config.
    /// Recomputed and re-emitted whenever room membership changes or a
    /// peer reports new capabilities. Frontend reconciles encoder +
    /// decoder by tearing down and reconfiguring with the new
    /// constraints. Backend owns the policy — CLI/TUI frontends inherit
    /// the same `SessionVideoConfig` payload without re-running any
    /// negotiation themselves.
    #[serde(rename_all = "camelCase")]
    VideoSessionConfig {
        community_id: String,
        channel_id: String,
        config: SessionVideoConfig,
    },
    /// Phase 3 — the negotiator found NO encoder codec every peer can
    /// decode (`negotiate_session_config` returned `None` with a
    /// non-empty local encode set). Emitted once per
    /// compatible→incompatible transition (latched — membership churn
    /// while incompatible does not re-emit). `peers` lists the
    /// pseudonyms whose decode sets blocked every local encode codec.
    /// Voice is unaffected; the frontend surfaces a toast.
    #[serde(rename_all = "camelCase")]
    VideoCodecIncompatible {
        community_id: String,
        channel_id: String,
        peers: Vec<String>,
    },
    /// Phase 4 — backend bitrate policy output (AIMD over receiver
    /// FrameAck/BandwidthEstimate feedback, audio reserve subtracted).
    /// The frontend encoder follows this target; the pacer rate moves
    /// with it on the backend.
    #[serde(rename_all = "camelCase")]
    VideoBitrateTarget {
        community_id: String,
        channel_id: String,
        kbps: u32,
    },
    /// The Linux-native capture session died asynchronously (camera
    /// unplugged, pipeline failure) — the panel reverts the camera
    /// toggle and surfaces the message.
    #[serde(rename_all = "camelCase")]
    NativeVideoError {
        community_id: String,
        channel_id: String,
        message: String,
    },
    /// Phase F — a gossiped video envelope failed signature or shape
    /// verification at the receive boundary. Surfaced to the UI so the
    /// asymmetric-drop case (one peer rejects, the other doesn't) is
    /// observable from frontend signals instead of grep.
    ///
    /// `communityId` / `senderPseudonym` may be the sentinel string
    /// `"<unknown>"` when the envelope failed to deserialize before
    /// those routing fields could be read.
    #[serde(rename_all = "camelCase")]
    VideoEnvelopeRejected {
        community_id: String,
        sender_pseudonym: String,
        reason: String,
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
    /// Architecture §6.2 — per-phase "dial-in" progress for the
    /// self-sovereign join. Emitted before and after each gated join
    /// phase (governance snapshot → invite decode → slot claim → presence
    /// → open records → watch) so the UI renders a stepper instead of a
    /// single spinner. `status` is one of `started` / `done` / `failed` /
    /// `timedOut`.
    #[serde(rename_all = "camelCase")]
    JoinProgress {
        community_id: String,
        stage: String,
        status: String,
    },
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
        /// Name carried by the join handshake — render without waiting
        /// for the registry scan.
        display_name: Option<String>,
    },
    /// A member left a voice channel.
    #[serde(rename_all = "camelCase")]
    VoiceLeave {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// Full voice-channel roster sent to a joiner so it sees everyone
    /// already present (decoupled from MEK-decrypt). §10.1/§10.5.
    #[serde(rename_all = "camelCase")]
    VoiceRoster {
        community_id: String,
        channel_id: String,
        participants: Vec<VoiceRosterParticipantEvent>,
    },
    /// Local three-way voice join handshake progressed
    /// ("seen" | "connected"). `peer`/`display_name` identify the
    /// member whose evidence drove the transition.
    #[serde(rename_all = "camelCase")]
    VoiceJoinHandshake {
        community_id: String,
        channel_id: String,
        state: String,
        peer: Option<String>,
        display_name: Option<String>,
    },
    /// A joiner completed its handshake (transport-ready) — the UI
    /// renders them solid instead of pending.
    #[serde(rename_all = "camelCase")]
    VoicePeerConfirmed {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// Media-ready gate state for the active voice/video session
    /// (WebRTC "transport before RTP" analog). Emitted on every
    /// (ready, reason) transition; `reason` names the next blocker
    /// ("handshake-announced" → … → "ready"). The frontend disables
    /// camera/screen-share until `ready` and the backend hard-rejects
    /// video egress.
    #[serde(rename_all = "camelCase")]
    VoiceMediaReady {
        community_id: String,
        channel_id: String,
        ready: bool,
        reason: String,
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
