//! Roles are Host calls (plan E3.1 F1): the Host's role operations
//! (`host/adapters/governance/role_ops.rs`) check, write and merge them on
//! both modes, and the windows learn the merged result from the governance
//! projection.

use rekindle_ipc::protocol::MemberRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::display::RoleDisplay;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::governance::ExclusionGroupEdit;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// What a role action answers when it ran.
type Ran = Done<serde_json::Value>;

/// The community's live roles (`IpcRequest::Community(CommunityRequest::RoleList)`).
#[tauri::command]
pub async fn get_roles(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<RoleDisplay>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleList {
            community: community_id,
        })),
    )
    .await
}

/// Create a role; answers its id (`IpcRequest::Community(CommunityRequest::RoleCreate)`).
#[tauri::command]
pub async fn create_role(
    community_id: String,
    name: String,
    color: u32,
    permissions: u64,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<u32>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleCreate {
            community: community_id,
            name,
            permissions,
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
        })),
    )
    .await
}

/// Edit a role (`IpcRequest::Community(CommunityRequest::RoleUpdate)`); `None` fields are unchanged.
#[tauri::command]
pub async fn edit_role(
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions: Option<u64>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    self_assignable: Option<bool>,
    exclusion_group: ExclusionGroupEdit,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleUpdate {
            community: community_id,
            role_id,
            name,
            permissions,
            color,
            position,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
        })),
    )
    .await
}

/// Archive a role (`IpcRequest::Community(CommunityRequest::RoleDelete)`).
#[tauri::command]
pub async fn delete_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleDelete {
            community: community_id,
            role_id,
        })),
    )
    .await
}

/// Grant a member a role, additively (`IpcRequest::Community(CommunityRequest::RoleAssign)`).
#[tauri::command]
pub async fn assign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleAssign {
            community: community_id,
            member_pseudonym: pseudonym_key,
            role_id,
        })),
    )
    .await
}

/// Take a role from a member (`IpcRequest::Community(CommunityRequest::RoleUnassign)`).
#[tauri::command]
pub async fn unassign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleUnassign {
            community: community_id,
            member_pseudonym: pseudonym_key,
            role_id,
        })),
    )
    .await
}

/// Take a self-assignable role (`IpcRequest::Community(CommunityRequest::RoleSelfAssign)`).
#[tauri::command]
pub async fn self_assign_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleSelfAssign {
            community: community_id,
            role_id,
        })),
    )
    .await
}

/// Drop a self-assignable role we hold (`IpcRequest::Community(CommunityRequest::RoleSelfUnassign)`).
#[tauri::command]
pub async fn self_unassign_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleSelfUnassign {
            community: community_id,
            role_id,
        })),
    )
    .await
}
