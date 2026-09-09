//! Governance dispatch handlers: roles, moderation, invites.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{adapter, state_error, DaemonContext};

// ── Roles ───────────────────────────────────────────────────────────────

/// Every live role, from merged governance.
///
/// The v1.0 manifest roles subkey is gone: `RoleAssign` always wrote
/// `RoleAssignment` entries while create/update/delete wrote the
/// manifest, so a daemon-created role had no `RoleDefinition` for
/// `compute_permissions` to find and the assignment conferred nothing —
/// on either track.
pub(crate) fn role_displays(
    ctx: &DaemonContext,
    community_id: &str,
) -> Vec<rekindle_types::display::RoleDisplay> {
    let Some(gov) = ctx.community_runtime.governance_state(community_id) else {
        return Vec::new();
    };
    let mut out: Vec<rekindle_types::display::RoleDisplay> = gov
        .roles
        .iter()
        .map(|(id, role)| rekindle_types::display::RoleDisplay {
            id: id.to_legacy_u32(),
            name: role.name.clone(),
            color: role.color,
            permissions: role.permissions,
            position: i32::try_from(role.position).unwrap_or(i32::MAX),
            // All four live in the merged CRDT state and were simply
            // not being read, so the CLI rendered every role as
            // non-hoisted, non-mentionable and ungrouped.
            hoist: role.hoist,
            mentionable: role.mentionable,
            self_assignable: role.self_assignable,
            exclusion_group: role.exclusion_group.clone(),
        })
        .collect();
    out.sort_by_key(|r| r.id);
    out
}

pub(crate) fn handle_role_list(
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
    // From merged governance, not the v1.0 manifest roles subkey.
    // `RoleAssign` has always written `RoleAssignment` entries while
    // create/update/delete wrote the manifest, so a daemon-created role
    // had no `RoleDefinition` for `compute_permissions` to find and the
    // assignment conferred nothing — on either track.
    IpcResponse::ok(&role_displays(ctx, &membership.governance_key))
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
    // Still gated on being attached: the entry write goes to the DHT,
    // and failing here is a clearer answer than a deep write error.
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    // `position` is allocated by the runtime from the merged role table
    // so two peers creating concurrently do not collide; the caller's
    // hint is ignored rather than silently honoured on one track only.
    let _ = spec.position;
    match rekindle_governance_runtime::roles::create_role(
        &adapter(ctx),
        &membership.governance_key,
        name.clone(),
        spec.color,
        spec.permissions,
        false,
        false,
        false,
        None,
    )
    .await
    {
        Ok(role_id) => IpcResponse::ok(&serde_json::json!({
            "id": role_id, "name": name, "permissions": spec.permissions,
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
    // Still gated on being attached: the entry write goes to the DHT,
    // and failing here is a clearer answer than a deep write error.
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let patch = rekindle_governance_runtime::roles::RoleSnapshotPatch {
        name: name.map(ToOwned::to_owned),
        permissions,
        color,
        position: None,
        hoist: None,
        mentionable: None,
        self_assignable: None,
        exclusion_group: rekindle_governance_runtime::roles::ExclusionGroupEdit::Unchanged,
    };
    match rekindle_governance_runtime::roles::edit_role(
        &adapter(ctx),
        &membership.governance_key,
        role_id,
        patch,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "id": role_id, "name": name })),
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
    // Still gated on being attached: the entry write goes to the DHT,
    // and failing here is a clearer answer than a deep write error.
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    // Archived, not deleted: `RoleArchived` is the CRDT's tombstone, so
    // a peer that had merged the definition drops it on the next merge
    // rather than keeping a role nobody else can see.
    match rekindle_governance_runtime::roles::delete_role(
        &adapter(ctx),
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
    // Still gated on being attached: the entry write goes to the DHT,
    // and failing here is a clearer answer than a deep write error.
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    // Mints the encrypted secrets record and writes `InviteCreated`.
    // The previous path appended an `InviteEntry` to the v1.0 manifest
    // with `encrypted_secrets: None`, so the invite carried no slot seed
    // and the join failed with "invite has no secrets pointer" — and
    // with no governance entry, revocation, expiry and the per-inviter
    // quota never applied to it either.
    match rekindle_governance_runtime::invites::create_invite(
        &adapter(ctx),
        &membership.governance_key,
        max_uses,
        expires_seconds,
    )
    .await
    {
        Ok(invite) => IpcResponse::ok(&serde_json::json!({
            "invite_code": invite.code,
            "code_hash": invite.code_hash,
            "secrets_record_key": invite.secrets_record_key,
            "invite_id": hex::encode(invite.invite_id),
            "expires_at": invite.expires_at,
            // The joiner needs all three parts, so hand back the link
            // rather than making every frontend assemble it.
            "invite_link": format!(
                "rekindle://invite/{}/{}/{}",
                membership.governance_key, invite.secrets_record_key, invite.code
            ),
        })),
        Err(e) => IpcResponse::error(500, format!("invite create: {e}")),
    }
}

pub(crate) fn handle_invite_list(
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
    // From merged governance. Revocation is a `InviteRevoked` tombstone
    // the merge applies, so an entry present here is one no peer has
    // revoked — the manifest list could not say that.
    let now = rekindle_utils::timestamp_secs();
    let invites: Vec<serde_json::Value> = ctx
        .community_runtime
        .governance_state(&membership.governance_key)
        .map(|gov| {
            gov.invites
                .iter()
                .filter(|(_, inv)| inv.expires_at.is_none_or(|exp| exp > now))
                .map(|(invite_id, inv)| {
                    serde_json::json!({
                        "invite_id": hex::encode(invite_id),
                        "code_hash": inv.code_hash,
                        "max_uses": inv.max_uses,
                        "expires_at": inv.expires_at,
                        "secrets_record_key": inv.secrets_record_key,
                        "created_by": hex::encode(inv.creator_pseudonym.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    IpcResponse::ok(&invites)
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
    // Still gated on being attached: the entry write goes to the DHT,
    // and failing here is a clearer answer than a deep write error.
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
    let Some(invite_id) = hex::decode(invite_code)
        .ok()
        .and_then(|b| <[u8; 16]>::try_from(b).ok())
    else {
        return IpcResponse::error(
            400,
            "revoke takes the 16-byte invite id from `invite list`, not the raw code — \
             the code is never published, only its hash",
        );
    };
    match rekindle_governance_runtime::invites::revoke_invite(
        &adapter(ctx),
        &membership.governance_key,
        invite_id,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "revoked": hex::encode(invite_id) })),
        Err(e) => IpcResponse::error(500, format!("invite revoke: {e}")),
    }
}
