use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;

use super::types::CommunityRoleDto;
use crate::services::community_role_handlers_runtime::{
    create_role_handler_inner, edit_role_handler_inner,
};
use crate::services::community_role_runtime::{
    assign_role_with_check_inner, delete_role_with_check_inner, get_roles_inner,
    self_assign_role_inner, self_unassign_role_inner, unassign_role_with_check_inner,
};

pub use crate::services::community_role_handlers_runtime::ExclusionGroupEdit;

/// Get all role definitions for a community from the merged governance state.
#[tauri::command]
pub async fn get_roles(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<CommunityRoleDto>, String> {
    get_roles_inner(state.inner(), &community_id)
}

/// Create a new role in a community.
///
/// `permissions` is accepted as a string to avoid JavaScript `Number` precision loss
/// on u64 values above `2^53 - 1`.
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches RoleDefinition shape"
)]
pub async fn create_role(
    community_id: String,
    name: String,
    color: u32,
    permissions: String,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<u32, String> {
    create_role_handler_inner(
        state.inner(),
        pool.inner(),
        community_id,
        name,
        color,
        permissions,
        hoist,
        mentionable,
        self_assignable,
        exclusion_group,
    )
    .await
}

/// Edit an existing role in a community.
///
/// `permissions` is accepted as a string to avoid JavaScript `Number` precision loss
/// on u64 values above `2^53 - 1`.
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches edit_role partial-update payload"
)]
pub async fn edit_role(
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions: Option<String>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    self_assignable: Option<bool>,
    exclusion_group: Option<ExclusionGroupEdit>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    edit_role_handler_inner(
        state.inner(),
        pool.inner(),
        community_id,
        role_id,
        name,
        color,
        permissions,
        position,
        hoist,
        mentionable,
        self_assignable,
        exclusion_group,
    )
    .await
}

/// Delete a role from a community.
#[tauri::command]
pub async fn delete_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    delete_role_with_check_inner(state.inner(), pool.inner(), community_id, role_id).await
}

/// Assign a role to a member (additive — does not remove other roles).
#[tauri::command]
pub async fn assign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    assign_role_with_check_inner(
        state.inner(),
        pool.inner(),
        community_id,
        pseudonym_key,
        role_id,
    )
    .await
}

/// Remove a role from a member.
#[tauri::command]
pub async fn unassign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    unassign_role_with_check_inner(
        state.inner(),
        pool.inner(),
        community_id,
        pseudonym_key,
        role_id,
    )
    .await
}

/// Assign a self-assignable role to the current member.
#[tauri::command]
pub async fn self_assign_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    self_assign_role_inner(state.inner(), pool.inner(), community_id, role_id).await
}

/// Remove a self-assignable role from the current member.
#[tauri::command]
pub async fn self_unassign_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    self_unassign_role_inner(state.inner(), pool.inner(), community_id, role_id).await
}
