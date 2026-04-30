use serde::{Deserialize, Serialize};

/// Events streamed from Rust to the frontend for community operations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum CommunityEvent {
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
}

/// Event info DTO for frontend consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInfoDto {
    pub id: String,
    pub title: String,
    pub description: String,
    pub creator_pseudonym: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<String>,
    pub max_attendees: Option<u32>,
    pub created_at: u64,
    pub status: String,
    pub rsvps: Vec<EventRsvpInfoDto>,
}

/// RSVP entry DTO for frontend consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRsvpInfoDto {
    pub pseudonym_key: String,
    pub status: String,
}

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
        }
    }
}

/// Thread info DTO for frontend consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfoDto {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub starter_message_id: String,
    pub creator_pseudonym: String,
    pub created_at: u64,
    pub archived: bool,
    pub auto_archive_seconds: u32,
    pub last_message_at: u64,
    pub message_count: u32,
}

/// Game server info DTO for frontend consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerInfoDto {
    pub id: String,
    pub game_id: String,
    pub label: String,
    pub address: String,
    pub added_by: String,
    pub created_at: u64,
}

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
        }
    }
}
