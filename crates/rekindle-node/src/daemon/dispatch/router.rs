//! The `dispatch()` router: matches every `IpcRequest` variant to its
//! domain handler, plus the audit-logging helper it calls before routing.

use crate::host::gate::Acknowledged;
use crate::host::shell_hooks::EndReason;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::{
    admin, channel, community, friends, governance, identity, keys, lifecycle, own, status, voice,
    Caller, DaemonContext,
};

/// Dispatch a validated, gated IPC request to its domain handler
/// (`HostApi::call` validated it and ran the intent gate); `acknowledged`
/// is what committing its review acknowledged (plan E2.3).
///
/// Returns an `IpcResponse` for every request — no request goes unanswered.
/// Every `IpcRequest` variant is explicitly matched; there is no catch-all arm.
pub async fn dispatch(
    ctx: &DaemonContext,
    request: IpcRequest,
    caller: &Caller,
    acknowledged: Acknowledged,
) -> IpcResponse {
    let state = ctx.lifecycle.state();

    // Audit: log every request before dispatch (best-effort, non-blocking).
    audit_request(ctx, &request);

    route(ctx, state, request, caller, acknowledged).await
}

/// Route an audited, validated request to its domain handler: one arm per
/// `IpcRequest` variant.
async fn route(
    ctx: &DaemonContext,
    state: crate::daemon::DaemonState,
    request: IpcRequest,
    caller: &Caller,
    acknowledged: Acknowledged,
) -> IpcResponse {
    match request {
        // ── Lifecycle (any state) ────────────────────────────────
        IpcRequest::Status => status::handle_status(ctx, state).await,
        IpcRequest::Unlock {
            identity,
            passphrase,
        } => lifecycle::handle_unlock(ctx, state, caller, &identity, &passphrase).await,
        // Both end the unlock (its teardown announces offline first); the
        // reason tells the shell which window to show.
        IpcRequest::Lock => lifecycle::handle_lock(ctx, state, EndReason::Lock).await,
        IpcRequest::Logout => lifecycle::handle_lock(ctx, state, EndReason::Logout).await,
        IpcRequest::Shutdown => lifecycle::handle_shutdown(ctx),

        // ── Identity ─────────────────────────────────────────────
        IpcRequest::IdentityCreate {
            display_name,
            passphrase,
        } => {
            identity::handle_create(ctx, state, caller, display_name.as_deref(), &passphrase).await
        }
        IpcRequest::IdentityList => identity::handle_list(ctx),
        IpcRequest::IdentityShow => identity::handle_show(ctx),
        IpcRequest::IdentityExport { passphrase } => {
            identity::handle_export(ctx, state, &passphrase)
        }
        IpcRequest::IdentityImport {
            archive,
            passphrase,
        } => identity::handle_import(ctx, &archive, &passphrase).await,
        IpcRequest::IdentityRotate { passphrase } => {
            identity::handle_rotate(ctx, state, &passphrase).await
        }
        // Protected: the gate checked the passphrase (plan D3.3).
        IpcRequest::IdentityDelete { identity } => {
            identity::handle_delete(ctx, state, &identity).await
        }
        IpcRequest::IdentityWipe => identity::handle_wipe(ctx, state).await,
        // The gate answers these in `HostApi::call`; they never route.
        IpcRequest::IntentCommit { .. }
        | IpcRequest::IntentCancel { .. }
        | IpcRequest::Undo { .. } => IpcResponse::error(
            400,
            "intent commits and undos are answered by the host's gate",
        ),

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

        // ── Friends, DMs and session resets (plan E2.1) ─────────
        request @ (IpcRequest::FriendAddFromCode { .. }
        | IpcRequest::FriendAccept { .. }
        | IpcRequest::FriendReject { .. }
        | IpcRequest::FriendRemove { .. }
        | IpcRequest::FriendList
        | IpcRequest::FriendRequests
        | IpcRequest::FriendCancelRequest { .. }
        | IpcRequest::FriendRetryHandshake { .. }
        | IpcRequest::FriendAddFromInvite { .. }
        | IpcRequest::FriendInviteCreate
        | IpcRequest::FriendCode
        | IpcRequest::FriendTokenRotate
        | IpcRequest::FriendInviteList
        | IpcRequest::FriendInviteCancel { .. }
        | IpcRequest::FriendBlock { .. }
        | IpcRequest::FriendUnblock { .. }
        | IpcRequest::FriendBlockedList
        | IpcRequest::FriendSetNickname { .. }
        | IpcRequest::FriendGroupCreate { .. }
        | IpcRequest::FriendGroupRename { .. }
        | IpcRequest::FriendGroupMove { .. }
        | IpcRequest::FriendGroupDelete { .. }
        | IpcRequest::SessionResetStart { .. }
        | IpcRequest::SessionResetAccept { .. }
        | IpcRequest::IdentityRotationAccept { .. }
        | IpcRequest::IdentityRotationDecline { .. }
        | IpcRequest::SessionResetDecline { .. }
        | IpcRequest::IdentitySafetyNumber { .. }
        | IpcRequest::IdentityMarkVerified { .. }
        | IpcRequest::IdentityClearVerification { .. }
        | IpcRequest::DmSend { .. }
        | IpcRequest::DmTyping { .. }
        | IpcRequest::DmInbox { .. }
        | IpcRequest::DmHistory { .. }
        | IpcRequest::DmMarkRead { .. }
        | IpcRequest::DmResend { .. }) => friends::route(ctx, state, request, acknowledged).await,

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
            governance::handle_invite_list(ctx, state, &community)
        }
        IpcRequest::InviteRevoke {
            community,
            invite_code,
        } => governance::handle_invite_revoke(ctx, state, &community, &invite_code).await,

        // ── Keys ─────────────────────────────────────────────────
        IpcRequest::PrekeyStatus => keys::handle_prekey_status(ctx, state).await,
        IpcRequest::MekList { community } => keys::handle_mek_list(ctx, state, &community),
        IpcRequest::MekRotate { community, channel } => {
            keys::handle_mek_rotate(ctx, state, &community, &channel)
        }
        IpcRequest::MekRequest {
            community,
            channel,
            generation,
        } => keys::handle_mek_request(ctx, state, &community, &channel, generation),

        // ── Our presence, profile, game, audit log, links ────────
        request @ (IpcRequest::PresenceSet { .. }
        | IpcRequest::PresenceSetMessage { .. }
        | IpcRequest::PresenceGet
        | IpcRequest::ProfileSetNickname { .. }
        | IpcRequest::SetReadReceiptsEnabled { .. }
        | IpcRequest::ProfileSetAvatar { .. }
        | IpcRequest::AvatarGet { .. }
        | IpcRequest::GamePresenceSet { .. }
        | IpcRequest::GamePresenceClear
        | IpcRequest::AuditVerify
        | IpcRequest::AuditExport { .. }
        | IpcRequest::LinkOpen { .. }) => own::route(ctx, state, request).await,

        // ── Voice ────────────────────────────────────────────────
        IpcRequest::VoiceJoin {
            community,
            channel,
            muted,
            deafened,
        } => voice::handle_voice_join(ctx, state, &community, &channel, muted, deafened),
        IpcRequest::VoiceLeave => voice::handle_voice_leave(ctx, state),

        // ── Admin / Network ──────────────────────────────────────
        // Subscribe/Unsubscribe are handled server-side in the IPC bus router.
        // They never reach daemon dispatch — the server intercepts them to
        // register filters in the EventRouter before routing to daemon.
        // If they arrive here, something bypassed the server layer.
        IpcRequest::Subscribe { .. } | IpcRequest::Unsubscribe { .. } => IpcResponse::error(
            400,
            "subscriptions are handled by the IPC bus server, not daemon dispatch",
        ),
        IpcRequest::NetworkStatus => admin::handle_network_status(ctx, state),
        IpcRequest::NetworkPeers => admin::handle_network_peers(ctx, state),
        IpcRequest::AgentRegister {
            name,
            agent_type,
            capabilities,
        } => admin::handle_agent_register(ctx, caller, &name, agent_type, &capabilities).await,
        IpcRequest::AgentRevoke { name } => admin::handle_agent_revoke(ctx, &name).await,
        IpcRequest::PolicyReload => admin::handle_policy_reload(ctx),
    }
}

/// Record a request in the unlocked identity's audit chain (`host::audit`):
/// the variant name only, never a field, so no key, name or message content
/// reaches the trail. Best-effort, and nothing is recorded while locked —
/// no identity database is open, and an unauthenticated caller must not
/// write into one (plan D1).
fn audit_request(ctx: &DaemonContext, request: &IpcRequest) {
    let Ok(db) = ctx.db.current() else {
        return;
    };
    let Some(owner) = ctx
        .session
        .read()
        .as_ref()
        .map(|s| s.identity.public_key_hex.clone())
    else {
        return;
    };
    crate::host::audit::append(
        &ctx.audit_chain,
        &db,
        &ctx.vault,
        &owner,
        rekindle_audit::AuditKind::Request,
        serde_json::json!({ "request": request.name() }),
        ctx.clock.now_ms_i64(),
    );
}
