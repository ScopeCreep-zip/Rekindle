//! Governance dispatch handlers: roles, moderation, invites.

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use super::{adapter, state_error, DaemonContext};

// ── Roles (the Host's, `host/adapters/governance/role_ops.rs`) ─────────

pub(crate) use crate::host::adapters::governance::role_ops::NewRole;
use crate::host::adapters::governance::role_ops::RoleRefusal;
pub(crate) use rekindle_governance_runtime::roles::RoleSnapshotPatch;

/// Every live role, from merged governance.
pub(crate) fn role_displays(
    ctx: &DaemonContext,
    community_id: &str,
) -> Vec<rekindle_types::display::RoleDisplay> {
    ctx.communities
        .governance_state(community_id)
        .map(|g| crate::host::adapters::governance::role_ops::role_displays(&g))
        .unwrap_or_default()
}

fn refused(refusal: &RoleRefusal) -> IpcResponse {
    IpcResponse::error(u32::from(refusal.code()), refusal.to_string())
}

/// The community a role write is about, once we may write and are attached
/// (the entry goes to the DHT).
fn writable(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> Result<String, IpcResponse> {
    if !state.can_write() {
        return Err(state_error(state, "write"));
    }
    ctx.require_transport()?;
    Ok(ctx.resolve_community(community)?.governance_key)
}

pub(crate) fn handle_role_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    match ctx.resolve_community(community) {
        Ok(m) => IpcResponse::ok(&adapter(ctx).role_list(&m.governance_key)),
        Err(e) => e,
    }
}

/// Create a role; answers its id.
pub(crate) async fn handle_role_create(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    role: NewRole,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).role_create(&key, role).await {
        Ok(role_id) => IpcResponse::ok(&role_id),
        Err(r) => refused(&r),
    }
}

pub(crate) async fn handle_role_update(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    role_id: u32,
    patch: RoleSnapshotPatch,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).role_edit(&key, role_id, patch).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "id": role_id })),
        Err(r) => refused(&r),
    }
}

/// Archived, not deleted: `RoleArchived` is the CRDT's tombstone.
pub(crate) async fn handle_role_delete(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    role_id: u32,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).role_delete(&key, role_id).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "deleted": role_id })),
        Err(r) => refused(&r),
    }
}

/// Grant (`grant`) or revoke a member's role: a `RoleAssignment` or
/// `RoleUnassignment` entry every peer merges and validates.
pub(crate) async fn handle_role_grant(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    member_pseudonym: &str,
    role_id: u32,
    grant: bool,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    let host = adapter(ctx);
    let done = if grant {
        host.role_assign(&key, member_pseudonym, role_id).await
    } else {
        host.role_unassign(&key, member_pseudonym, role_id).await
    };
    match done {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "assigned": grant, "role_id": role_id })),
        Err(r) => refused(&r),
    }
}

/// Take (`take`) or drop a self-assignable role ourselves.
pub(crate) async fn handle_role_self(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    role_id: u32,
    take: bool,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).role_self(&key, role_id, take).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "assigned": take, "role_id": role_id })),
        Err(r) => refused(&r),
    }
}

// ── Moderation (the Host's, `host/adapters/governance/moderation.rs`) ──

use crate::host::adapters::governance::moderation::ModerationRefusal;

fn moderation_refused(refusal: &ModerationRefusal) -> IpcResponse {
    IpcResponse::error(u32::from(refusal.code()), refusal.to_string())
}

/// Remove a member without barring return.
pub(crate) fn handle_kick(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).remove_member(&key, target) {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "kicked": target })),
        Err(r) => moderation_refused(&r),
    }
}

pub(crate) async fn handle_ban(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
    reason: Option<&str>,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).ban(&key, target, reason).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "banned": target, "reason": reason })),
        Err(r) => moderation_refused(&r),
    }
}

pub(crate) async fn handle_unban(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).unban(&key, target).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "unbanned": target })),
        Err(r) => moderation_refused(&r),
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
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx)
        .timeout(&key, target, duration_seconds, reason)
        .await
    {
        Ok(()) => IpcResponse::ok(
            &serde_json::json!({ "timedOut": target, "durationSeconds": duration_seconds }),
        ),
        Err(r) => moderation_refused(&r),
    }
}

pub(crate) async fn handle_timeout_remove(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    target: &str,
) -> IpcResponse {
    let key = match writable(ctx, state, community) {
        Ok(k) => k,
        Err(e) => return e,
    };
    match adapter(ctx).remove_timeout(&key, target).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "timeoutRemoved": target })),
        Err(r) => moderation_refused(&r),
    }
}

/// Current bans, read from merged CRDT state and named from the roster.
pub(crate) async fn handle_ban_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).ban_list(&membership.governance_key).await {
        Ok(bans) => IpcResponse::ok(&bans),
        Err(e) => IpcResponse::error(500, e),
    }
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
    // Still gated on being attached: the entry write goes to the DHT,
    // and failing here is a clearer answer than a deep write error.
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .create_invite(&membership.governance_key, max_uses, expires_seconds)
        .await
    {
        Ok(created) => IpcResponse::ok(&created),
        Err(e) => IpcResponse::error(500, e),
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
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).list_invites(&membership.governance_key).await {
        Ok(invites) => IpcResponse::ok(&invites),
        Err(e) => IpcResponse::error(500, e),
    }
}

pub(crate) async fn handle_invite_revoke(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    invite_id: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    // Identified by invite id, not by the raw code: the code is never
    // published (only its hash is), so a revoker who was not the minter
    // has no way to supply it. `InviteList` returns the ids.
    let Some(id) = hex::decode(invite_id)
        .ok()
        .and_then(|b| <[u8; 16]>::try_from(b).ok())
    else {
        return IpcResponse::error(400, "revoke takes the 16-byte invite id from `invite list`");
    };
    match adapter(ctx)
        .revoke_invite(&membership.governance_key, id)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "revoked": invite_id })),
        Err(e) => IpcResponse::error(500, e),
    }
}
