//! Phase 23.D.15 — thin facade. All role-mutation orchestration ported
//! into `rekindle_governance_runtime::roles` parameterised over
//! `GovernanceRuntimeDeps`. This module wraps the crate-side
//! orchestrators with `GovernanceAdapter` construction so the
//! existing command callers (which take `&SharedState` + `&DbPool`)
//! keep their call shape unchanged.

use std::sync::Arc;

use rekindle_governance_runtime::roles::{ExclusionGroupEdit, RoleSnapshotPatch};
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use tauri::Manager;

use crate::commands::community::helpers::require_permission;
use crate::commands::community::types::CommunityRoleDto;
use crate::db::DbPool;
use crate::services::governance_adapter::GovernanceAdapter;
use crate::state::SharedState;

pub fn get_roles_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<CommunityRoleDto>, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    Ok(community.roles.iter().map(CommunityRoleDto::from).collect())
}

pub async fn self_assign_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    role_id: u32,
) -> Result<(), String> {
    let pseudonym_key = resolve_self_assignable_pseudonym(state, &community_id, role_id)?;
    assign_role_inner(state, pool, &community_id, &pseudonym_key, role_id).await
}

pub async fn self_unassign_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    role_id: u32,
) -> Result<(), String> {
    let pseudonym_key = resolve_self_assignable_pseudonym(state, &community_id, role_id)?;
    unassign_role_inner(state, pool, &community_id, &pseudonym_key, role_id).await
}

pub async fn delete_role_with_check_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    role_id: u32,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_ROLES)?;
    delete_role_inner(state, pool, community_id, role_id).await
}

pub async fn assign_role_with_check_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_ROLES)?;
    assign_role_inner(state, pool, &community_id, &pseudonym_key, role_id).await
}

pub async fn unassign_role_with_check_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_ROLES)?;
    unassign_role_inner(state, pool, &community_id, &pseudonym_key, role_id).await
}

fn build_adapter(state: &SharedState, pool: &DbPool) -> Result<GovernanceAdapter, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle not initialized".to_string())?;
    let _: tauri::State<'_, DbPool> = app_handle.state();
    Ok(GovernanceAdapter::new(
        Arc::clone(state),
        app_handle.clone(),
        pool.clone(),
    ))
}

pub async fn assign_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> Result<(), String> {
    let adapter = build_adapter(state, pool)?;
    rekindle_governance_runtime::roles::assign_role(&adapter, community_id, pseudonym_key, role_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn unassign_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> Result<(), String> {
    let adapter = build_adapter(state, pool)?;
    rekindle_governance_runtime::roles::unassign_role(
        &adapter,
        community_id,
        pseudonym_key,
        role_id,
    )
    .await
    .map_err(|e| e.to_string())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches RoleDefinition shape"
)]
pub async fn create_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    name: String,
    color: u32,
    permissions_u64: u64,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<String>,
) -> Result<u32, String> {
    let adapter = build_adapter(state, pool)?;
    rekindle_governance_runtime::roles::create_role(
        &adapter,
        &community_id,
        name,
        color,
        permissions_u64,
        hoist,
        mentionable,
        self_assignable,
        exclusion_group,
    )
    .await
    .map_err(|e| e.to_string())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches edit_role partial-update payload"
)]
pub async fn edit_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions_u64: Option<u64>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    self_assignable: Option<bool>,
    exclusion_group: ExclusionGroupEdit,
) -> Result<(), String> {
    let adapter = build_adapter(state, pool)?;
    let patch = RoleSnapshotPatch {
        name,
        color,
        permissions: permissions_u64,
        position,
        hoist,
        mentionable,
        self_assignable,
        exclusion_group,
    };
    rekindle_governance_runtime::roles::edit_role(&adapter, &community_id, role_id, patch)
        .await
        .map_err(|e| e.to_string())
}

pub async fn delete_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    role_id: u32,
) -> Result<(), String> {
    let adapter = build_adapter(state, pool)?;
    rekindle_governance_runtime::roles::delete_role(&adapter, &community_id, role_id)
        .await
        .map_err(|e| e.to_string())
}

pub fn resolve_self_assignable_pseudonym(
    state: &SharedState,
    community_id: &str,
    role_id: u32,
) -> Result<String, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle not initialized".to_string())?;
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let adapter =
        GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.inner().clone());
    rekindle_governance_runtime::roles::resolve_self_assignable_pseudonym(
        &adapter,
        community_id,
        role_id,
    )
    .map_err(|e| e.to_string())
}
