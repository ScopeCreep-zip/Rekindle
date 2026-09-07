//! Governance dispatch handlers: roles, moderation, invites.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{adapter, state_error, DaemonContext};

// ── Roles ───────────────────────────────────────────────────────────────

pub(crate) async fn handle_role_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match rekindle_transport::operations::roles::list_roles(&transport, &membership.governance_key)
        .await
    {
        Ok(roles) => IpcResponse::ok(&roles),
        Err(e) => IpcResponse::error(500, format!("role list: {e}")),
    }
}

/// Attributes for a new role, grouped so the dispatch handler threads a single
/// spec instead of four independent scalars.
pub(crate) struct RoleSpec<'a> {
    pub name: &'a str,
    pub permissions: u64,
    pub color: u32,
    pub position: i32,
}

pub(crate) async fn handle_role_create(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    spec: RoleSpec<'_>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let name = match validation::validate_name(spec.name, "Role") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match rekindle_transport::operations::roles::create_role(
        &transport,
        &membership.governance_key,
        &name,
        spec.permissions,
        spec.color,
        spec.position,
    )
    .await
    {
        Ok(role) => IpcResponse::ok(&serde_json::json!({
            "id": role.id, "name": role.name, "permissions": role.permissions,
        })),
        Err(e) => IpcResponse::error(500, format!("role create: {e}")),
    }
}

pub(crate) async fn handle_role_update(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    role_id: u32,
    name: Option<&str>,
    permissions: Option<u64>,
    color: Option<u32>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Some(n) = name {
        if let Err(e) = validation::validate_name(n, "Role") {
            return e;
        }
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match rekindle_transport::operations::roles::update_role(
        &transport,
        &membership.governance_key,
        role_id,
        name,
        permissions,
        color,
    )
    .await
    {
        Ok(role) => IpcResponse::ok(&serde_json::json!({
            "id": role.id, "name": role.name,
        })),
        Err(e) => IpcResponse::error(500, format!("role update: {e}")),
    }
}

pub(crate) async fn handle_role_delete(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    role_id: u32,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match rekindle_transport::operations::roles::delete_role(
        &transport,
        &membership.governance_key,
        role_id,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "deleted": role_id })),
        Err(e) => IpcResponse::error(500, format!("role delete: {e}")),
    }
}

/// Grant a role.
///
/// Writes a `RoleAssignment` governance entry, which every peer merges
/// and validates. The previous path rewrote the *shared* registry
/// member index — one member editing another's row in a record whose
/// `o_cnt: 0` schema gives nobody a writer credential for it, and a
/// second source of truth against the CRDT the desktop already used
/// (`community_role_runtime.rs`). Two stores for one rule, and only one
/// of them was v2.0.
pub(crate) async fn handle_role_assign(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    member_pseudonym: &str,
    role_id: u32,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }
    match rekindle_governance_runtime::roles::assign_role(
        &adapter(ctx),
        community,
        member_pseudonym,
        role_id,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "assigned": true, "role_id": role_id })),
        Err(e) => IpcResponse::error(500, format!("role assign: {e}")),
    }
}

/// Revoke a role. Same reasoning as [`handle_role_assign`].
pub(crate) async fn handle_role_unassign(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    member_pseudonym: &str,
    role_id: u32,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }
    match rekindle_governance_runtime::roles::unassign_role(
        &adapter(ctx),
        community,
        member_pseudonym,
        role_id,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "unassigned": true, "role_id": role_id })),
        Err(e) => IpcResponse::error(500, format!("role unassign: {e}")),
    }
}

// ── Moderation ──────────────────────────────────────────────────────────

/// Remove a member without barring return.
///
/// Now writes a governance entry via the shared moderation module. It
/// previously went through `transport::operations::moderation`, which
/// maintained a **separate** bans/member list on a governance subkey
/// rather than the CRDT — so a member banned from the daemon was not
/// banned on the desktop, and vice versa. Two incompatible stores for
/// one rule.
pub(crate) async fn handle_kick(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }
    match rekindle_governance_runtime::moderation::kick_member(&adapter(ctx), community, target)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "kicked": target })),
        Err(e) => IpcResponse::error(500, format!("kick failed: {e}")),
    }
}

pub(crate) async fn handle_ban(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
    reason: Option<&str>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }
    match rekindle_governance_runtime::moderation::ban_member(
        &adapter(ctx),
        community,
        target,
        reason,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "banned": target, "reason": reason })),
        Err(e) => IpcResponse::error(500, format!("ban failed: {e}")),
    }
}

pub(crate) async fn handle_unban(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }
    match rekindle_governance_runtime::moderation::unban_member(&adapter(ctx), community, target)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "unbanned": target })),
        Err(e) => IpcResponse::error(500, format!("unban failed: {e}")),
    }
}

pub(crate) async fn handle_timeout(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
    duration_seconds: u64,
    reason: Option<&str>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }
    match rekindle_governance_runtime::moderation::timeout_member(
        &adapter(ctx),
        community,
        target,
        duration_seconds,
        reason,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(
            &serde_json::json!({ "timedOut": target, "durationSeconds": duration_seconds }),
        ),
        Err(e) => IpcResponse::error(500, format!("timeout failed: {e}")),
    }
}

/// Current bans, read from merged CRDT state.
///
/// The old implementation read a bespoke bans list off a governance
/// subkey, which no longer receives writes.
pub(crate) fn handle_ban_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }
    let bans: Vec<String> = ctx
        .community_runtime
        .governance_state(community)
        .map(|s| s.bans.iter().map(|p| hex::encode(p.0)).collect())
        .unwrap_or_default();
    IpcResponse::ok(&serde_json::json!({ "bans": bans }))
}

pub(crate) async fn handle_invite_create(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    max_uses: u32,
    expires_seconds: Option<u64>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match rekindle_transport::operations::invites::create_invite(
        &transport,
        &membership.governance_key,
        &membership.pseudonym_key,
        max_uses,
        expires_seconds,
    )
    .await
    {
        Ok(code) => IpcResponse::ok(&serde_json::json!({ "invite_code": code })),
        Err(e) => IpcResponse::error(500, format!("invite create: {e}")),
    }
}

pub(crate) async fn handle_invite_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match rekindle_transport::operations::invites::list_invites(
        &transport,
        &membership.governance_key,
    )
    .await
    {
        Ok(invites) => IpcResponse::ok(&invites),
        Err(e) => IpcResponse::error(500, format!("invite list: {e}")),
    }
}

pub(crate) async fn handle_invite_revoke(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    invite_code: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match rekindle_transport::operations::invites::revoke_invite(
        &transport,
        &membership.governance_key,
        invite_code,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "revoked": true })),
        Err(e) => IpcResponse::error(500, format!("invite revoke: {e}")),
    }
}
