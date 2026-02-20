use serde::Serialize;

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
        new_generation: u64,
    },
    /// We were kicked from a community (our pseudonym was removed by an admin).
    #[serde(rename_all = "camelCase")]
    Kicked { community_id: String },
    /// Role definitions changed (created, edited, deleted, reordered).
    #[serde(rename_all = "camelCase")]
    RolesChanged {
        community_id: String,
        roles: Vec<RoleDto>,
    },
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
}

/// Role DTO for frontend consumption (mirrors protocol's `RoleDto`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDto {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
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
        }
    }
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
        }
    }
}
