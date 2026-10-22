//! The community arm of the `dispatch()` router (plan E3.1): a community,
//! channel, role, moderation, invite or channel-key request to its domain
//! handler.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use crate::daemon::dispatch::{channel, community, governance, keys, DaemonContext};
use crate::daemon::DaemonState;

/// Route a community request: one arm per variant.
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
) -> IpcResponse {
    match request {
        // ── Community ────────────────────────────────────────────
        IpcRequest::CommunityCreate {
            name,
            description,
            approval_required,
        } => community::handle_create(ctx, state, &name, &description, approval_required).await,
        IpcRequest::CommunityJoin { invite } => community::handle_join(ctx, state, &invite).await,
        IpcRequest::CommunityLeave { governance_key } => {
            community::handle_leave(ctx, state, &governance_key).await
        }
        IpcRequest::CommunityList => community::handle_list(ctx, state),
        IpcRequest::CommunityInfo { governance_key } => {
            community::handle_info(ctx, state, &governance_key)
        }
        IpcRequest::CommunityUpdateInfo {
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
        IpcRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        } => community::handle_approve(ctx, state, &governance_key, &member_pseudonym).await,
        IpcRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            reason,
        } => {
            community::handle_reject(ctx, state, &governance_key, &member_pseudonym, &reason).await
        }
        IpcRequest::CommunityPendingMembers { governance_key } => {
            community::handle_pending_members(ctx, state, &governance_key)
        }
        IpcRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym,
        } => {
            community::handle_transfer_ownership(ctx, state, &governance_key, &new_owner_pseudonym)
                .await
        }

        // ── Channel ──────────────────────────────────────────────
        request @ (IpcRequest::ChannelList { .. }
        | IpcRequest::ChannelCreate { .. }
        | IpcRequest::ChannelDelete { .. }
        | IpcRequest::ChannelUpdate { .. }
        | IpcRequest::ChannelSend { .. }
        | IpcRequest::ChannelHistory { .. }) => channel::route(ctx, state, request).await,

        // ── Governance (roles, moderation, invites) ──────────────
        IpcRequest::RoleList { community } => governance::handle_role_list(ctx, state, &community),
        IpcRequest::RoleCreate {
            community,
            name,
            permissions,
            color,
            position,
        } => {
            governance::handle_role_create(
                ctx,
                state,
                &community,
                governance::RoleSpec {
                    name: &name,
                    permissions,
                    color,
                    position,
                },
            )
            .await
        }
        IpcRequest::RoleUpdate {
            community,
            role_id,
            name,
            permissions,
            color,
        } => {
            governance::handle_role_update(
                ctx,
                state,
                &community,
                role_id,
                name.as_deref(),
                permissions,
                color,
            )
            .await
        }
        IpcRequest::RoleDelete { community, role_id } => {
            governance::handle_role_delete(ctx, state, &community, role_id).await
        }
        IpcRequest::RoleAssign {
            community,
            member_pseudonym,
            role_id,
        } => {
            governance::handle_role_assign(ctx, state, &community, &member_pseudonym, role_id).await
        }
        IpcRequest::RoleUnassign {
            community,
            member_pseudonym,
            role_id,
        } => {
            governance::handle_role_unassign(ctx, state, &community, &member_pseudonym, role_id)
                .await
        }
        IpcRequest::Kick {
            community,
            target_pseudonym,
        } => governance::handle_kick(ctx, state, &community, &target_pseudonym).await,
        IpcRequest::Ban {
            community,
            target_pseudonym,
            reason,
        } => {
            governance::handle_ban(ctx, state, &community, &target_pseudonym, reason.as_deref())
                .await
        }
        IpcRequest::Unban {
            community,
            target_pseudonym,
        } => governance::handle_unban(ctx, state, &community, &target_pseudonym).await,
        IpcRequest::Timeout {
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
        IpcRequest::BanList { community } => governance::handle_ban_list(ctx, state, &community),
        IpcRequest::InviteCreate {
            community,
            max_uses,
            expires_seconds,
        } => {
            governance::handle_invite_create(ctx, state, &community, max_uses, expires_seconds)
                .await
        }
        IpcRequest::InviteList { community } => {
            governance::handle_invite_list(ctx, state, &community).await
        }
        IpcRequest::InviteRevoke {
            community,
            invite_code,
        } => governance::handle_invite_revoke(ctx, state, &community, &invite_code).await,

        // ── Channel keys ─────────────────────────────────────────
        IpcRequest::MekList { community } => keys::handle_mek_list(ctx, state, &community),
        IpcRequest::MekRotate { community, channel } => {
            keys::handle_mek_rotate(ctx, state, &community, &channel)
        }
        IpcRequest::MekRequest {
            community,
            channel,
            generation,
        } => keys::handle_mek_request(ctx, state, &community, &channel, generation),
        other => unreachable!("{} is not a community request", other.name()),
    }
}
