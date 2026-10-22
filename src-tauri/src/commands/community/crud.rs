use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;

use crate::state::SharedState;
use rekindle_types::dto::community::{CommunityCreated, CommunityView};
use rekindle_vault::VaultHandle;

/// The joined communities (`IpcRequest::CommunityList`).
#[tauri::command]
pub async fn get_communities(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<rekindle_types::display::CommunityOverview>>, CommandError> {
    host_call::call(&state, IpcRequest::CommunityList).await
}

/// Every joined community as its window renders it
/// (`IpcRequest::CommunityViews`).
#[tauri::command]
pub async fn get_community_details(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<CommunityView>>, CommandError> {
    host_call::call(&state, IpcRequest::CommunityViews).await
}

/// Create a community (`IpcRequest::CommunityCreate`).
///
/// `approval_required` picks the genesis admission mode and is
/// immutable afterwards: the CRDT honours `AdmissionPolicy` only as the
/// genesis entry, so a later one is ignored by every honest peer.
#[tauri::command]
pub async fn create_community(
    name: String,
    approval_required: Option<bool>,
    state: State<'_, SharedState>,
) -> Result<Done<CommunityCreated>, CommandError> {
    let request = IpcRequest::CommunityCreate {
        name,
        description: String::new(),
        approval_required: approval_required.unwrap_or(false),
    };
    host_call::call(&state, request).await
}

/// Join a community from its invite link. Returns the community id.
#[tauri::command]
pub async fn join_community(
    invite_url: String,
    state: State<'_, SharedState>,
    keystore_handle: State<'_, VaultHandle>,
) -> Result<String, String> {
    let pool = state.db.current()?;
    let link = rekindle_types::invite::InviteLink::parse(&invite_url).map_err(|e| e.to_string())?;
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    crate::services::community_lifecycle_runtime::join_community_inner(
        state.inner(),
        &pool,
        keystore_handle.inner(),
        &link,
    )
    .await?;
    Ok(link.governance_key.into_string())
}

/// Leave a community (`IpcRequest::CommunityLeave`), reviewed with its
/// leave escalations.
#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::CommunityLeave {
        governance_key: community_id,
    };
    host_call::call(&state, request).await
}

/// Edit the community's name, description, icon or banner; `None` keeps
/// the merged value (`IpcRequest::CommunityUpdateInfo`).
#[tauri::command]
pub async fn update_community_info(
    community_id: String,
    name: Option<String>,
    description: Option<String>,
    icon_hash: Option<String>,
    banner_hash: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::CommunityUpdateInfo {
        governance_key: community_id,
        name,
        description,
        icon_hash,
        banner_hash,
    };
    host_call::call(&state, request).await
}
