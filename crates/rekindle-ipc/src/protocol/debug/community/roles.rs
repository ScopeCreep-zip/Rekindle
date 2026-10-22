//! The role and game-server requests' `Debug` (plan E3.1).

use super::super::super::CommunityRequest;

impl CommunityRequest {
    /// The role and game-server requests' `Debug`.
    pub(super) fn fmt_roles_and_servers(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::RoleList { community } => f
                .debug_struct("RoleList")
                .field("community", community)
                .finish(),
            Self::RoleCreate {
                community,
                name,
                permissions,
                color,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            } => f
                .debug_struct("RoleCreate")
                .field("community", community)
                .field("name", name)
                .field("permissions", permissions)
                .field("color", color)
                .field("hoist", hoist)
                .field("mentionable", mentionable)
                .field("self_assignable", self_assignable)
                .field("exclusion_group", exclusion_group)
                .finish(),
            Self::RoleUpdate {
                community,
                role_id,
                name,
                permissions,
                color,
                position,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            } => f
                .debug_struct("RoleUpdate")
                .field("community", community)
                .field("role_id", role_id)
                .field("name", name)
                .field("permissions", permissions)
                .field("color", color)
                .field("position", position)
                .field("hoist", hoist)
                .field("mentionable", mentionable)
                .field("self_assignable", self_assignable)
                .field("exclusion_group", exclusion_group)
                .finish(),
            Self::RoleDelete { community, role_id } => f
                .debug_struct("RoleDelete")
                .field("community", community)
                .field("role_id", role_id)
                .finish(),
            Self::ChannelMarkRead {
                community,
                channel,
                up_to,
            } => f
                .debug_struct("ChannelMarkRead")
                .field("community", community)
                .field("channel", channel)
                .field("up_to", up_to)
                .finish(),
            Self::UnreadCounts { community } => f
                .debug_struct("UnreadCounts")
                .field("community", community)
                .finish(),
            Self::GameServerList { community } => f
                .debug_struct("GameServerList")
                .field("community", community)
                .finish(),
            Self::GameServerAdd {
                community,
                game_id,
                label,
                address,
            } => f
                .debug_struct("GameServerAdd")
                .field("community", community)
                .field("game_id", game_id)
                .field("label", label)
                .field("address", address)
                .finish(),
            Self::GameServerRemove {
                community,
                server_id,
            } => f
                .debug_struct("GameServerRemove")
                .field("community", community)
                .field("server_id", server_id)
                .finish(),
            Self::RoleAssign {
                community,
                member_pseudonym,
                role_id,
            } => f
                .debug_struct("RoleAssign")
                .field("community", community)
                .field("member_pseudonym", member_pseudonym)
                .field("role_id", role_id)
                .finish(),
            Self::RoleUnassign {
                community,
                member_pseudonym,
                role_id,
            } => f
                .debug_struct("RoleUnassign")
                .field("community", community)
                .field("member_pseudonym", member_pseudonym)
                .field("role_id", role_id)
                .finish(),
            Self::RoleSelfAssign { community, role_id } => f
                .debug_struct("RoleSelfAssign")
                .field("community", community)
                .field("role_id", role_id)
                .finish(),
            Self::RoleSelfUnassign { community, role_id } => f
                .debug_struct("RoleSelfUnassign")
                .field("community", community)
                .field("role_id", role_id)
                .finish(),
            other => unreachable!("{} is not a role or game-server request", other.name()),
        }
    }
}
