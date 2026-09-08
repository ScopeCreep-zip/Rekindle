//! The `dispatch()` router: matches every `IpcRequest` variant to its
//! domain handler, plus the audit-logging helper it calls before routing.

use crate::ipc::protocol::{IpcRequest, IpcResponse};

use super::{
    admin, channel, community, governance, identity, keys, lifecycle, presence, social,
    DaemonContext,
};

/// Dispatch an IPC request to the appropriate domain handler.
///
/// Returns an `IpcResponse` for every request — no request goes unanswered.
/// Every `IpcRequest` variant is explicitly matched; there is no catch-all arm.
pub async fn dispatch(ctx: &DaemonContext, request: IpcRequest) -> IpcResponse {
    let state = ctx.lifecycle.state();

    // Audit: log every request before dispatch (best-effort, non-blocking).
    audit_request(ctx, &request);

    match request {
        // ── Lifecycle (any state) ────────────────────────────────
        IpcRequest::Status => lifecycle::handle_status(ctx, state),
        IpcRequest::Unlock { passphrase } => {
            lifecycle::handle_unlock(ctx, state, &passphrase).await
        }
        IpcRequest::Lock => lifecycle::handle_lock(ctx),
        IpcRequest::Shutdown => lifecycle::handle_shutdown(ctx),

        // ── Identity ─────────────────────────────────────────────
        IpcRequest::IdentityCreate { display_name } => {
            identity::handle_create(ctx, state, &display_name).await
        }
        IpcRequest::IdentityShow => identity::handle_show(ctx, state),
        IpcRequest::IdentityExport => identity::handle_export(ctx, state),
        IpcRequest::IdentityRotate => identity::handle_rotate(ctx, state).await,
        IpcRequest::IdentityDestroy { confirmation } => {
            identity::handle_destroy(ctx, state, &confirmation).await
        }
        IpcRequest::IdentityWipe { confirmation } => {
            identity::handle_wipe(ctx, state, &confirmation).await
        }

        // ── Community ────────────────────────────────────────────
        IpcRequest::CommunityCreate { name, description } => {
            community::handle_create(ctx, state, &name, &description).await
        }
        IpcRequest::CommunityJoin { invite } => community::handle_join(ctx, state, &invite).await,
        IpcRequest::CommunityLeave { governance_key } => {
            community::handle_leave(ctx, state, &governance_key).await
        }
        IpcRequest::CommunityList => community::handle_list(ctx, state).await,
        IpcRequest::CommunityInfo { governance_key } => {
            community::handle_info(ctx, state, &governance_key).await
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
        IpcRequest::ChannelList { community } => channel::handle_list(ctx, state, &community),
        IpcRequest::ChannelCreate {
            community,
            name,
            kind,
            category,
            topic,
            slowmode_seconds,
        } => {
            channel::handle_create(
                ctx,
                state,
                &community,
                &name,
                &kind,
                category.as_deref(),
                topic.as_deref(),
                slowmode_seconds,
            )
            .await
        }
        IpcRequest::ChannelDelete {
            community,
            channel_id,
        } => channel::handle_delete(ctx, state, &community, &channel_id).await,
        IpcRequest::ChannelUpdate {
            community,
            channel_id,
            name,
            topic,
            slowmode_seconds,
        } => {
            channel::handle_update(
                ctx,
                state,
                &community,
                &channel_id,
                channel::ChannelUpdate {
                    name: name.as_deref(),
                    topic: topic.as_deref(),
                    slowmode_seconds,
                },
            )
            .await
        }
        IpcRequest::ChannelSend {
            community,
            channel,
            body,
            reply_to,
        } => channel::handle_send(ctx, state, &community, &channel, &body, reply_to).await,
        IpcRequest::ChannelHistory {
            community,
            channel,
            limit,
        } => channel::handle_history(ctx, state, &community, &channel, limit).await,

        // ── Social (friends + DMs) ───────────────────────────────
        IpcRequest::FriendAdd { target, message } => {
            social::handle_friend_add(ctx, state, &target, &message).await
        }
        IpcRequest::FriendAccept { public_key } => {
            social::handle_friend_accept(ctx, state, &public_key).await
        }
        IpcRequest::FriendReject { public_key } => {
            social::handle_friend_reject(ctx, state, &public_key).await
        }
        IpcRequest::FriendRemove { public_key } => {
            social::handle_friend_remove(ctx, state, &public_key).await
        }
        IpcRequest::FriendList => social::handle_friend_list(ctx, state).await,
        IpcRequest::FriendRequests => social::handle_friend_requests(ctx, state),
        IpcRequest::DmSend { peer_key, body } => {
            social::handle_dm_send(ctx, state, &peer_key, &body).await
        }
        IpcRequest::DmTyping { peer_key, typing } => {
            social::handle_dm_typing(ctx, state, &peer_key, typing).await
        }
        IpcRequest::DmInbox { limit } => social::handle_dm_inbox(ctx, state, limit).await,

        // ── Governance (roles, moderation, invites) ──────────────
        IpcRequest::RoleList { community } => {
            governance::handle_role_list(ctx, state, &community).await
        }
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

        // ── Keys ─────────────────────────────────────────────────
        IpcRequest::MekList { community } => keys::handle_mek_list(ctx, state, &community),
        IpcRequest::MekRotate { community, channel } => {
            keys::handle_mek_rotate(ctx, state, &community, &channel)
        }
        IpcRequest::MekRequest {
            community,
            channel,
            generation,
        } => keys::handle_mek_request(ctx, state, &community, &channel, generation),
        IpcRequest::PrekeyReplenish => keys::handle_prekey_replenish(ctx, state).await,

        // ── Presence + Voice ─────────────────────────────────────
        IpcRequest::PresenceSet { status, message } => {
            presence::handle_set(ctx, state, &status, message.as_deref()).await
        }
        IpcRequest::GamePresenceSet {
            game_name,
            game_id,
            elapsed_seconds,
            server_address,
        } => {
            presence::handle_game_set(
                ctx,
                state,
                &game_name,
                game_id,
                elapsed_seconds,
                server_address.as_deref(),
            )
            .await
        }
        IpcRequest::GamePresenceClear => presence::handle_game_clear(ctx, state).await,
        IpcRequest::VoiceJoin {
            community,
            channel,
            muted,
            deafened,
        } => presence::handle_voice_join(ctx, state, &community, &channel, muted, deafened).await,
        IpcRequest::VoiceLeave => presence::handle_voice_leave(ctx, state),

        // ── Admin / Network ──────────────────────────────────────
        // Subscribe/Unsubscribe are handled server-side in the IPC bus router.
        // They never reach daemon dispatch — the server intercepts them to
        // register filters in the EventRouter before routing to daemon.
        // If they arrive here, something bypassed the server layer.
        IpcRequest::Subscribe { .. } => IpcResponse::error(
            400,
            "subscribe must be handled by the IPC bus server, not daemon dispatch",
        ),
        IpcRequest::Unsubscribe { .. } => IpcResponse::error(
            400,
            "unsubscribe must be handled by the IPC bus server, not daemon dispatch",
        ),
        IpcRequest::NetworkStatus => admin::handle_network_status(ctx, state),
        IpcRequest::NetworkPeers => admin::handle_network_peers(ctx, state),
        IpcRequest::AgentRegister {
            name,
            agent_type,
            capabilities,
        } => admin::handle_agent_register(ctx, &name, agent_type, &capabilities).await,
        IpcRequest::AgentRevoke { name } => admin::handle_agent_revoke(ctx, &name).await,
        IpcRequest::PolicyReload => admin::handle_policy_reload(ctx),
    }
}

/// Log an IPC request to the BLAKE3 hash-chained audit log.
///
/// Best-effort: audit failures are logged but never block the request.
/// The audit entry includes the request type and security-relevant context
/// but never the payload body (no message content in the audit trail).
fn audit_request(ctx: &DaemonContext, request: &IpcRequest) {
    let event_type = format!("{request:?}");
    // Truncate to just the variant name for the audit log (no field data)
    let event_name = event_type
        .split_once(' ')
        .or_else(|| event_type.split_once('{'))
        .map_or(event_type.as_str(), |(name, _)| name)
        .trim();

    let mut guard = ctx.audit.lock();
    if let Some(ref mut logger) = *guard {
        if let Err(e) = logger.append(
            event_name.as_bytes(),
            None, // sender name filled by server layer
            crate::ipc::message::SecurityLevel::Open,
            event_name.to_string(),
            None,
        ) {
            tracing::debug!(error = %e, event = event_name, "audit log write failed (non-fatal)");
        }
    }
}
