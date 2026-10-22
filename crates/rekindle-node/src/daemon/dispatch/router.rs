//! The `dispatch()` router: matches every `IpcRequest` variant to its
//! domain handler, plus the audit-logging helper it calls before routing.

use crate::host::gate::Acknowledged;
use crate::host::shell_hooks::EndReason;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::{admin, friends, identity, keys, lifecycle, own, status, voice, Caller, DaemonContext};

mod community;

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

        // ── Community, channels, governance and channel keys (plan E3.1) ──
        request @ (IpcRequest::CommunityCreate { .. }
        | IpcRequest::CommunityJoin { .. }
        | IpcRequest::CommunityLeave { .. }
        | IpcRequest::CommunityList
        | IpcRequest::CommunityViews
        | IpcRequest::CommunityMembers { .. }
        | IpcRequest::CommunityInfo { .. }
        | IpcRequest::CommunityUpdateInfo { .. }
        | IpcRequest::CommunityApprove { .. }
        | IpcRequest::CommunityReject { .. }
        | IpcRequest::CommunityPendingMembers { .. }
        | IpcRequest::CommunityTransferOwnership { .. }
        | IpcRequest::ChannelList { .. }
        | IpcRequest::ChannelCreate { .. }
        | IpcRequest::ChannelDelete { .. }
        | IpcRequest::ChannelUpdate { .. }
        | IpcRequest::ChannelRename { .. }
        | IpcRequest::ChannelMove { .. }
        | IpcRequest::ChannelReorder { .. }
        | IpcRequest::ChannelSetTopic { .. }
        | IpcRequest::ChannelSetForumTags { .. }
        | IpcRequest::ChannelOverwriteSet { .. }
        | IpcRequest::ChannelOverwriteDelete { .. }
        | IpcRequest::CategoryCreate { .. }
        | IpcRequest::CategoryDelete { .. }
        | IpcRequest::CategoryRename { .. }
        | IpcRequest::CategoryReorder { .. }
        | IpcRequest::ChannelSend { .. }
        | IpcRequest::ChannelHistory { .. }
        | IpcRequest::RoleList { .. }
        | IpcRequest::RoleCreate { .. }
        | IpcRequest::RoleUpdate { .. }
        | IpcRequest::RoleDelete { .. }
        | IpcRequest::GameServerList { .. }
        | IpcRequest::GameServerAdd { .. }
        | IpcRequest::GameServerRemove { .. }
        | IpcRequest::RoleAssign { .. }
        | IpcRequest::RoleUnassign { .. }
        | IpcRequest::RoleSelfAssign { .. }
        | IpcRequest::RoleSelfUnassign { .. }
        | IpcRequest::Kick { .. }
        | IpcRequest::Ban { .. }
        | IpcRequest::Unban { .. }
        | IpcRequest::Timeout { .. }
        | IpcRequest::BanList { .. }
        | IpcRequest::InviteCreate { .. }
        | IpcRequest::InviteList { .. }
        | IpcRequest::InviteRevoke { .. }
        | IpcRequest::MekList { .. }
        | IpcRequest::MekRotate { .. }
        | IpcRequest::MekRequest { .. }) => community::route(ctx, state, request).await,

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
        | IpcRequest::SessionReinitiate { .. }
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
        | IpcRequest::OutboxRetry { .. }
        | IpcRequest::OutboxDiscard { .. }
        | IpcRequest::GroupDmCreate { .. }
        | IpcRequest::GroupDmAccept { .. }
        | IpcRequest::GroupDmDecline { .. }
        | IpcRequest::GroupDmLeave { .. }
        | IpcRequest::GroupDmSend { .. }
        | IpcRequest::GroupDmTyping { .. }
        | IpcRequest::GroupDmList
        | IpcRequest::GroupDmMessages { .. }) => {
            friends::route(ctx, state, request, acknowledged).await
        }

        // ── Keys ─────────────────────────────────────────────────
        IpcRequest::PrekeyStatus => keys::handle_prekey_status(ctx, state).await,

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
