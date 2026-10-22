//! The community, channel, MEK, role, moderation and invite requests'
//! `Debug` (plan E3.1): a message body shows its length only.

use super::super::CommunityRequest;

impl std::fmt::Debug for CommunityRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_community(f)
    }
}

impl CommunityRequest {
    /// The community requests' `Debug`.
    fn fmt_community(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelSend {
                community,
                channel,
                body,
                reply_to,
            } => f
                .debug_struct("ChannelSend")
                .field("community", community)
                .field("channel", channel)
                .field("body_len", &body.len())
                .field("reply_to", reply_to)
                .finish(),
            Self::CommunityCreate {
                name,
                description,
                approval_required,
            } => f
                .debug_struct("CommunityCreate")
                .field("name", name)
                .field("description", description)
                .field("approval_required", approval_required)
                .finish(),
            Self::CommunityJoin { invite } => f
                .debug_struct("CommunityJoin")
                .field("invite", invite)
                .finish(),
            Self::CommunityLeave { governance_key } => f
                .debug_struct("CommunityLeave")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityUpdateProfile { .. }
            | Self::CommunitySetAvatar { .. }
            | Self::CommunitySetBanner { .. }
            | Self::CommunityAvatarGet { .. }
            | Self::CommunityPolicyGet { .. }
            | Self::CommunityPolicySet { .. } => self.fmt_profile(f),
            Self::CommunityList => write!(f, "CommunityList"),
            Self::CommunityViews => write!(f, "CommunityViews"),
            Self::CommunityMembers { governance_key } => f
                .debug_struct("CommunityMembers")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityUpdateInfo {
                governance_key,
                name,
                description,
                icon_hash,
                banner_hash,
            } => f
                .debug_struct("CommunityUpdateInfo")
                .field("governance_key", governance_key)
                .field("name", &name.as_ref().map(String::len))
                .field("description", &description.as_ref().map(String::len))
                .field("icon_hash", icon_hash)
                .field("banner_hash", banner_hash)
                .finish(),
            Self::CommunityInfo { governance_key } => f
                .debug_struct("CommunityInfo")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityApprove {
                governance_key,
                member_pseudonym,
            } => f
                .debug_struct("CommunityApprove")
                .field("governance_key", governance_key)
                .field("member_pseudonym", member_pseudonym)
                .finish(),
            Self::CommunityReject {
                governance_key,
                member_pseudonym,
                reason,
            } => f
                .debug_struct("CommunityReject")
                .field("governance_key", governance_key)
                .field("member_pseudonym", member_pseudonym)
                .field("reason", reason)
                .finish(),
            Self::CommunityPendingMembers { governance_key } => f
                .debug_struct("CommunityPendingMembers")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityTransferOwnership {
                governance_key,
                new_owner_pseudonym,
            } => f
                .debug_struct("CommunityTransferOwnership")
                .field("governance_key", governance_key)
                .field("new_owner_pseudonym", new_owner_pseudonym)
                .finish(),
            Self::ChannelList { community } => f
                .debug_struct("ChannelList")
                .field("community", community)
                .finish(),
            Self::ChannelCreate {
                community,
                name,
                kind,
                category,
                topic,
                slowmode_seconds,
                parent_voice_channel,
            } => f
                .debug_struct("ChannelCreate")
                .field("community", community)
                .field("name", name)
                .field("kind", kind)
                .field("category", category)
                .field("topic", topic)
                .field("slowmode_seconds", slowmode_seconds)
                .field("parent_voice_channel", parent_voice_channel)
                .finish(),
            Self::ChannelDelete {
                community,
                channel_id,
            } => f
                .debug_struct("ChannelDelete")
                .field("community", community)
                .field("channel_id", channel_id)
                .finish(),
            Self::ChannelUpdate {
                community,
                channel_id,
                name,
                topic,
                slowmode_seconds,
            } => f
                .debug_struct("ChannelUpdate")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("name", name)
                .field("topic", topic)
                .field("slowmode_seconds", slowmode_seconds)
                .finish(),
            Self::ChannelRename { .. }
            | Self::ChannelMove { .. }
            | Self::ChannelReorder { .. }
            | Self::ChannelSetTopic { .. }
            | Self::ChannelSetForumTags { .. }
            | Self::ChannelOverwriteSet { .. }
            | Self::ChannelOverwriteDelete { .. }
            | Self::CategoryCreate { .. }
            | Self::CategoryDelete { .. }
            | Self::CategoryRename { .. }
            | Self::CategoryReorder { .. } => self.fmt_channel_admin(f),
            Self::ChannelHistory {
                community,
                channel,
                limit,
            } => f
                .debug_struct("ChannelHistory")
                .field("community", community)
                .field("channel", channel)
                .field("limit", limit)
                .finish(),
            Self::MekList { community } => f
                .debug_struct("MekList")
                .field("community", community)
                .finish(),
            Self::MekRotate { community, channel } => f
                .debug_struct("MekRotate")
                .field("community", community)
                .field("channel", channel)
                .finish(),
            Self::MekRequest {
                community,
                channel,
                generation,
            } => f
                .debug_struct("MekRequest")
                .field("community", community)
                .field("channel", channel)
                .field("generation", generation)
                .finish(),
            Self::RoleList { .. }
            | Self::RoleCreate { .. }
            | Self::RoleUpdate { .. }
            | Self::RoleDelete { .. }
            | Self::RoleAssign { .. }
            | Self::RoleUnassign { .. }
            | Self::RoleSelfAssign { .. }
            | Self::RoleSelfUnassign { .. }
            | Self::GameServerList { .. }
            | Self::GameServerAdd { .. }
            | Self::GameServerRemove { .. } => self.fmt_roles_and_servers(f),
            Self::Kick {
                community,
                target_pseudonym,
            } => f
                .debug_struct("Kick")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::Ban {
                community,
                target_pseudonym,
                reason,
            } => f
                .debug_struct("Ban")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .field("reason", reason)
                .finish(),
            Self::Unban {
                community,
                target_pseudonym,
            } => f
                .debug_struct("Unban")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::Timeout {
                community,
                target_pseudonym,
                duration_seconds,
                reason,
            } => f
                .debug_struct("Timeout")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .field("duration_seconds", duration_seconds)
                .field("reason", reason)
                .finish(),
            Self::TimeoutRemove {
                community,
                target_pseudonym,
            } => f
                .debug_struct("TimeoutRemove")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::BanList { community } => f
                .debug_struct("BanList")
                .field("community", community)
                .finish(),
            Self::InviteCreate {
                community,
                max_uses,
                expires_seconds,
            } => f
                .debug_struct("InviteCreate")
                .field("community", community)
                .field("max_uses", max_uses)
                .field("expires_seconds", expires_seconds)
                .finish(),
            Self::InviteList { community } => f
                .debug_struct("InviteList")
                .field("community", community)
                .finish(),
            Self::InviteRevoke {
                community,
                invite_code,
            } => f
                .debug_struct("InviteRevoke")
                .field("community", community)
                .field("invite_code", invite_code)
                .finish(),
        }
    }

    /// The channel and category administration requests' `Debug`.
    fn fmt_channel_admin(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelRename {
                community,
                channel_id,
                name,
            } => f
                .debug_struct("ChannelRename")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("name", name)
                .finish(),
            Self::ChannelMove {
                community,
                channel_id,
                category,
            } => f
                .debug_struct("ChannelMove")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("category", category)
                .finish(),
            Self::ChannelReorder {
                community,
                channel_ids,
            } => f
                .debug_struct("ChannelReorder")
                .field("community", community)
                .field("channel_ids", channel_ids)
                .finish(),
            Self::ChannelSetTopic {
                community,
                channel_id,
                topic,
            } => f
                .debug_struct("ChannelSetTopic")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("topic", topic)
                .finish(),
            Self::ChannelSetForumTags {
                community,
                channel_id,
                forum_tags,
            } => f
                .debug_struct("ChannelSetForumTags")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("forum_tags", forum_tags)
                .finish(),
            Self::ChannelOverwriteSet {
                community,
                channel_id,
                target_type,
                target_id,
                allow,
                deny,
            } => f
                .debug_struct("ChannelOverwriteSet")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("target_type", target_type)
                .field("target_id", target_id)
                .field("allow", allow)
                .field("deny", deny)
                .finish(),
            Self::ChannelOverwriteDelete {
                community,
                channel_id,
                target_type,
                target_id,
            } => f
                .debug_struct("ChannelOverwriteDelete")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("target_type", target_type)
                .field("target_id", target_id)
                .finish(),
            Self::CategoryCreate { community, name } => f
                .debug_struct("CategoryCreate")
                .field("community", community)
                .field("name", name)
                .finish(),
            Self::CategoryDelete {
                community,
                category_id,
            } => f
                .debug_struct("CategoryDelete")
                .field("community", community)
                .field("category_id", category_id)
                .finish(),
            Self::CategoryRename {
                community,
                category_id,
                name,
            } => f
                .debug_struct("CategoryRename")
                .field("community", community)
                .field("category_id", category_id)
                .field("name", name)
                .finish(),
            Self::CategoryReorder {
                community,
                category_ids,
            } => f
                .debug_struct("CategoryReorder")
                .field("community", community)
                .field("category_ids", category_ids)
                .finish(),
            other => unreachable!("{} is not a channel admin request", other.name()),
        }
    }

    /// The role and game-server requests' `Debug`.
    fn fmt_roles_and_servers(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    /// The profile, picture and policy requests' `Debug` (plan E3.1 D4):
    /// a picture and free text show their length only.
    fn fmt_profile(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommunitySetAvatar {
                community,
                image,
                kind,
            }
            | Self::CommunitySetBanner {
                community,
                image,
                kind,
            } => f
                .debug_struct(if matches!(self, Self::CommunitySetAvatar { .. }) {
                    "CommunitySetAvatar"
                } else {
                    "CommunitySetBanner"
                })
                .field("community", community)
                .field("image_len", &image.len())
                .field("kind", kind)
                .finish(),
            Self::CommunityUpdateProfile {
                community,
                bio,
                pronouns,
                theme_color,
                badges,
                avatar_ref,
                banner_ref,
            } => f
                .debug_struct("CommunityUpdateProfile")
                .field("community", community)
                .field("bio_len", &bio.as_ref().map(String::len))
                .field("pronouns_len", &pronouns.as_ref().map(String::len))
                .field("theme_color", theme_color)
                .field("badges", &badges.len())
                .field("avatar_ref", avatar_ref)
                .field("banner_ref", banner_ref)
                .finish(),
            Self::CommunityAvatarGet { community, hash } => f
                .debug_struct("CommunityAvatarGet")
                .field("community", community)
                .field("hash", hash)
                .finish(),
            Self::CommunityPolicyGet { community } => f
                .debug_struct("CommunityPolicyGet")
                .field("community", community)
                .finish(),
            Self::CommunityPolicySet {
                community,
                policy_text,
                max_joins_per_interval,
                join_interval_seconds,
            } => f
                .debug_struct("CommunityPolicySet")
                .field("community", community)
                .field("policy_text_len", &policy_text.as_ref().map(String::len))
                .field("max_joins_per_interval", max_joins_per_interval)
                .field("join_interval_seconds", join_interval_seconds)
                .finish(),
            _ => unreachable!("fmt_profile is called for the profile requests only"),
        }
    }
}
