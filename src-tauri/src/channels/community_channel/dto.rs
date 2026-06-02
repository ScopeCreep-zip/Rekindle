//! Data-transfer types carried inside [`super::CommunityEvent`] variants:
//! channel/category snapshots, role DTOs, and the `rekindle-types` aliases
//! for events/threads/game-servers, plus the `From` conversions that build
//! a `RoleDto` from the various in-crate and protocol role shapes.

use serde::{Deserialize, Serialize};

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
