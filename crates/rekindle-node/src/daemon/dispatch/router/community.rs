//! The community arm of the `dispatch()` router (plan E3.1): a community,
//! channel, role, moderation, invite or channel-key request to its domain
//! handler.

use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use crate::daemon::dispatch::{
    channel, community, community_profile, events, game_servers, governance, keys, DaemonContext,
};
use crate::daemon::DaemonState;

/// Route a community request: one arm per variant.
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    match request {
        // ── Community ────────────────────────────────────────────
        CommunityRequest::CommunityCreate {
            name,
            description,
            approval_required,
        } => community::handle_create(ctx, state, &name, &description, approval_required).await,
        CommunityRequest::CommunityJoin { invite } => {
            community::handle_join(ctx, state, &invite).await
        }
        CommunityRequest::CommunityLeave { governance_key } => {
            community::handle_leave(ctx, state, &governance_key).await
        }
        CommunityRequest::CommunityList => community::handle_list(ctx, state),
        CommunityRequest::CommunityViews => community::handle_views(ctx, state),
        CommunityRequest::CommunityMembers { governance_key } => {
            community::handle_members(ctx, state, &governance_key).await
        }
        CommunityRequest::CommunityInfo { governance_key } => {
            community::handle_info(ctx, state, &governance_key)
        }
        CommunityRequest::CommunityUpdateInfo {
            governance_key,
            name,
            description,
            icon_hash,
            banner_hash,
        } => {
            community::handle_update_info(
                ctx,
                state,
                &governance_key,
                crate::host::adapters::governance::InfoEdit {
                    name,
                    description,
                    icon_hash,
                    banner_hash,
                },
            )
            .await
        }
        CommunityRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        } => community::handle_approve(ctx, state, &governance_key, &member_pseudonym).await,
        CommunityRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            reason,
        } => {
            community::handle_reject(ctx, state, &governance_key, &member_pseudonym, &reason).await
        }
        CommunityRequest::CommunityPendingMembers { governance_key } => {
            community::handle_pending_members(ctx, state, &governance_key)
        }
        CommunityRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym,
        } => {
            community::handle_transfer_ownership(ctx, state, &governance_key, &new_owner_pseudonym)
                .await
        }

        // ── Channel ──────────────────────────────────────────────
        request @ (CommunityRequest::ChannelList { .. }
        | CommunityRequest::ChannelCreate { .. }
        | CommunityRequest::ChannelDelete { .. }
        | CommunityRequest::ChannelUpdate { .. }
        | CommunityRequest::ChannelRename { .. }
        | CommunityRequest::ChannelMove { .. }
        | CommunityRequest::ChannelReorder { .. }
        | CommunityRequest::ChannelSetTopic { .. }
        | CommunityRequest::ChannelSetForumTags { .. }
        | CommunityRequest::ChannelOverwriteSet { .. }
        | CommunityRequest::ChannelOverwriteDelete { .. }
        | CommunityRequest::CategoryCreate { .. }
        | CommunityRequest::CategoryDelete { .. }
        | CommunityRequest::CategoryRename { .. }
        | CommunityRequest::CategoryReorder { .. }
        | CommunityRequest::ChannelSend { .. }
        | CommunityRequest::ChannelHistory { .. }) => channel::route(ctx, state, request).await,

        // ── Governance (roles, moderation, invites) ──────────────
        // ── Profile, pictures and policy (plan E3.1 D4) ──────────
        CommunityRequest::CommunityUpdateProfile {
            community,
            bio,
            pronouns,
            theme_color,
            badges,
            avatar_ref,
            banner_ref,
        } => {
            let edit = community_profile::ProfileEdit {
                bio,
                pronouns,
                theme_color,
                badges,
                avatar_ref,
                banner_ref,
            };
            community_profile::handle_update_profile(ctx, state, &community, edit)
        }
        CommunityRequest::CommunitySetAvatar {
            community,
            image,
            kind,
        } => {
            let slot = community_profile::PictureSlot::Avatar;
            community_profile::handle_set_picture(ctx, state, &community, image, kind, slot).await
        }
        CommunityRequest::CommunitySetBanner {
            community,
            image,
            kind,
        } => {
            let slot = community_profile::PictureSlot::Banner;
            community_profile::handle_set_picture(ctx, state, &community, image, kind, slot).await
        }
        CommunityRequest::CommunityAvatarGet { community, hash } => {
            community_profile::handle_picture(ctx, state, &community, &hash).await
        }
        CommunityRequest::CommunityPolicyGet { community } => {
            community_profile::handle_policy(ctx, state, &community)
        }
        CommunityRequest::CommunityPolicySet {
            community,
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
        } => {
            let edit = community_profile::PolicyEdit {
                policy_text,
                max_joins_per_interval,
                join_interval_seconds,
            };
            community_profile::handle_set_policy(ctx, state, &community, edit).await
        }
        // ── Game servers (plan E3.1 H1) ──────────────────────────
        request @ (CommunityRequest::EventList { .. }
        | CommunityRequest::EventAttendees { .. }
        | CommunityRequest::EventCreate { .. }
        | CommunityRequest::EventEdit { .. }
        | CommunityRequest::EventCancel { .. }
        | CommunityRequest::EventDelete { .. }
        | CommunityRequest::EventRsvp { .. }) => events::handle(ctx, state, request).await,
        CommunityRequest::GameServerList { community } => {
            game_servers::handle_list(ctx, state, &community).await
        }
        CommunityRequest::GameServerAdd {
            community,
            game_id,
            label,
            address,
        } => {
            let server = game_servers::NewGameServer {
                game_id,
                label,
                address,
            };
            game_servers::handle_add(ctx, state, &community, server).await
        }
        CommunityRequest::GameServerRemove {
            community,
            server_id,
        } => game_servers::handle_remove(ctx, state, &community, &server_id).await,

        CommunityRequest::RoleList { community } => {
            governance::handle_role_list(ctx, state, &community)
        }
        CommunityRequest::RoleCreate {
            community,
            name,
            permissions,
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
        } => {
            let role = governance::NewRole {
                name,
                color,
                permissions,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            };
            governance::handle_role_create(ctx, state, &community, role).await
        }
        CommunityRequest::RoleUpdate {
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
        } => {
            let patch = governance::RoleSnapshotPatch {
                name,
                color,
                permissions,
                position,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            };
            governance::handle_role_update(ctx, state, &community, role_id, patch).await
        }
        CommunityRequest::RoleDelete { community, role_id } => {
            governance::handle_role_delete(ctx, state, &community, role_id).await
        }
        CommunityRequest::RoleAssign {
            community,
            member_pseudonym,
            role_id,
        } => {
            governance::handle_role_grant(ctx, state, &community, &member_pseudonym, role_id, true)
                .await
        }
        CommunityRequest::RoleUnassign {
            community,
            member_pseudonym,
            role_id,
        } => {
            governance::handle_role_grant(ctx, state, &community, &member_pseudonym, role_id, false)
                .await
        }
        CommunityRequest::RoleSelfAssign { community, role_id } => {
            governance::handle_role_self(ctx, state, &community, role_id, true).await
        }
        CommunityRequest::RoleSelfUnassign { community, role_id } => {
            governance::handle_role_self(ctx, state, &community, role_id, false).await
        }
        CommunityRequest::Kick {
            community,
            target_pseudonym,
        } => governance::handle_kick(ctx, state, &community, &target_pseudonym),
        CommunityRequest::Ban {
            community,
            target_pseudonym,
            reason,
        } => {
            governance::handle_ban(ctx, state, &community, &target_pseudonym, reason.as_deref())
                .await
        }
        CommunityRequest::Unban {
            community,
            target_pseudonym,
        } => governance::handle_unban(ctx, state, &community, &target_pseudonym).await,
        CommunityRequest::Timeout {
            community,
            target_pseudonym,
            duration_seconds,
            reason,
        } => {
            governance::handle_timeout(
                ctx,
                state,
                &community,
                &target_pseudonym,
                duration_seconds,
                reason.as_deref(),
            )
            .await
        }
        CommunityRequest::TimeoutRemove {
            community,
            target_pseudonym,
        } => governance::handle_timeout_remove(ctx, state, &community, &target_pseudonym).await,
        CommunityRequest::BanList { community } => {
            governance::handle_ban_list(ctx, state, &community).await
        }
        CommunityRequest::InviteCreate {
            community,
            max_uses,
            expires_seconds,
        } => {
            governance::handle_invite_create(ctx, state, &community, max_uses, expires_seconds)
                .await
        }
        CommunityRequest::InviteList { community } => {
            governance::handle_invite_list(ctx, state, &community).await
        }
        CommunityRequest::InviteRevoke {
            community,
            invite_code,
        } => governance::handle_invite_revoke(ctx, state, &community, &invite_code).await,

        // ── Channel keys ─────────────────────────────────────────
        CommunityRequest::MekList { community } => keys::handle_mek_list(ctx, state, &community),
        CommunityRequest::MekRotate { community, channel } => {
            keys::handle_mek_rotate(ctx, state, &community, &channel)
        }
        CommunityRequest::MekRequest {
            community,
            channel,
            generation,
        } => keys::handle_mek_request(ctx, state, &community, &channel, generation),
    }
}
