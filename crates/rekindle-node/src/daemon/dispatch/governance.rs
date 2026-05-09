//! Governance dispatch handlers: roles, moderation, invites.

use std::sync::Arc;
use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{DaemonContext, state_error};

// ── Roles ───────────────────────────────────────────────────────────────

pub(crate) async fn handle_role_list(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::roles::list_roles(&transport, &membership.governance_key).await {
        Ok(roles) => IpcResponse::ok(&roles),
        Err(e) => IpcResponse::error(500, format!("role list: {e}")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_role_create(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
    name: &str, permissions: u64, color: u32, position: i32,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let name = match validation::validate_name(name, "Role") { Ok(n) => n, Err(e) => return e };
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::roles::create_role(
        &transport, &membership.governance_key, &name, permissions, color, position,
    ).await {
        Ok(role) => IpcResponse::ok(&serde_json::json!({
            "id": role.id, "name": role.name, "permissions": role.permissions,
        })),
        Err(e) => IpcResponse::error(500, format!("role create: {e}")),
    }
}

pub(crate) async fn handle_role_update(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
    role_id: u32, name: Option<&str>, permissions: Option<u64>, color: Option<u32>,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    if let Some(n) = name {
        if let Err(e) = validation::validate_name(n, "Role") { return e; }
    }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::roles::update_role(
        &transport, &membership.governance_key, role_id, name, permissions, color,
    ).await {
        Ok(role) => IpcResponse::ok(&serde_json::json!({
            "id": role.id, "name": role.name,
        })),
        Err(e) => IpcResponse::error(500, format!("role update: {e}")),
    }
}

pub(crate) async fn handle_role_delete(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str, role_id: u32,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::roles::delete_role(
        &transport, &membership.governance_key, role_id,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "deleted": role_id })),
        Err(e) => IpcResponse::error(500, format!("role delete: {e}")),
    }
}

pub(crate) async fn handle_role_assign(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
    member_pseudonym: &str, role_id: u32,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::roles::assign_role(
        &transport, &membership.registry_key, member_pseudonym, role_id,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "assigned": true, "role_id": role_id })),
        Err(e) => IpcResponse::error(500, format!("role assign: {e}")),
    }
}

pub(crate) async fn handle_role_unassign(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
    member_pseudonym: &str, role_id: u32,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::roles::unassign_role(
        &transport, &membership.registry_key, member_pseudonym, role_id,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "unassigned": true, "role_id": role_id })),
        Err(e) => IpcResponse::error(500, format!("role unassign: {e}")),
    }
}

// ── Moderation ──────────────────────────────────────────────────────────

pub(crate) fn handle_kick(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str, target: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let _membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::moderation::build_kick_payload(target) {
        Ok(payload_bytes) => IpcResponse::ok(&serde_json::json!({
            "kicked": target,
            "gossip_payload_len": payload_bytes.len(),
        })),
        Err(e) => IpcResponse::error(500, format!("kick failed: {e}")),
    }
}

pub(crate) async fn handle_ban(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
    target: &str, reason: Option<&str>,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::moderation::ban_member(
        &transport, &membership.governance_key, target, reason, &membership.pseudonym_key,
    ).await {
        Ok(payload_bytes) => IpcResponse::ok(&serde_json::json!({
            "banned": target,
            "gossip_payload_len": payload_bytes.len(),
        })),
        Err(e) => IpcResponse::error(500, format!("ban failed: {e}")),
    }
}

pub(crate) async fn handle_unban(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str, target: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::moderation::unban_member(
        &transport, &membership.governance_key, target,
    ).await {
        Ok(_) => IpcResponse::ok(&serde_json::json!({ "unbanned": target })),
        Err(e) => IpcResponse::error(500, format!("unban failed: {e}")),
    }
}

pub(crate) fn handle_timeout(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
    target: &str, duration_seconds: u64, reason: Option<&str>,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let _membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::moderation::build_timeout_payload(
        target, duration_seconds, reason,
    ) {
        Ok(payload_bytes) => IpcResponse::ok(&serde_json::json!({
            "timed_out": target,
            "duration_seconds": duration_seconds,
            "gossip_payload_len": payload_bytes.len(),
        })),
        Err(e) => IpcResponse::error(500, format!("timeout failed: {e}")),
    }
}

pub(crate) async fn handle_ban_list(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::moderation::list_bans(&transport, &membership.governance_key).await {
        Ok(bans) => IpcResponse::ok(&bans),
        Err(e) => IpcResponse::error(500, format!("ban list: {e}")),
    }
}

// ── Invites ─────────────────────────────────────────────────────────────

pub(crate) async fn handle_invite_create(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
    max_uses: u32, expires_seconds: Option<u64>,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::invites::create_invite(
        &transport, &membership.governance_key, &membership.pseudonym_key,
        max_uses, expires_seconds,
    ).await {
        Ok(code) => IpcResponse::ok(&serde_json::json!({ "invite_code": code })),
        Err(e) => IpcResponse::error(500, format!("invite create: {e}")),
    }
}

pub(crate) async fn handle_invite_list(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::invites::list_invites(&transport, &membership.governance_key).await {
        Ok(invites) => IpcResponse::ok(&invites),
        Err(e) => IpcResponse::error(500, format!("invite list: {e}")),
    }
}

pub(crate) async fn handle_invite_revoke(
    ctx: &Arc<DaemonContext>, state: DaemonState, community: &str, invite_code: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };
    match rekindle_transport::operations::invites::revoke_invite(
        &transport, &membership.governance_key, invite_code,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "revoked": true })),
        Err(e) => IpcResponse::error(500, format!("invite revoke: {e}")),
    }
}
