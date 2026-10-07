use tauri::State;

use crate::keystore::KeystoreHandle;
use crate::services::community_views_runtime::{
    list_communities_inner, list_community_details_inner,
};
use crate::state::SharedState;

use super::types::CommunityDetail;
use crate::services::community_views_runtime::CommunityInfo;

#[tauri::command]
pub async fn get_communities(state: State<'_, SharedState>) -> Result<Vec<CommunityInfo>, String> {
    Ok(list_communities_inner(state.inner()))
}

#[tauri::command]
pub async fn get_community_details(
    state: State<'_, SharedState>,
) -> Result<Vec<CommunityDetail>, String> {
    Ok(list_community_details_inner(state.inner()))
}

/// Create a community.
///
/// `approval_required` picks the genesis admission mode and is
/// immutable afterwards: the CRDT honours `AdmissionPolicy` only as the
/// genesis entry, so a later one is ignored by every honest peer — not
/// even an administrator can flip a community from open to gated, which
/// is the point.
#[tauri::command]
pub async fn create_community(
    _app: tauri::AppHandle,
    name: String,
    approval_required: Option<bool>,
    state: State<'_, SharedState>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<String, String> {
    let pool = state.db.current()?;
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    crate::services::community_lifecycle_runtime::create_community_inner(
        state.inner(),
        &pool,
        keystore_handle.inner(),
        name,
        if approval_required.unwrap_or(false) {
            rekindle_types::governance::AdmissionMode::ApprovalRequired
        } else {
            rekindle_types::governance::AdmissionMode::Open
        },
    )
    .await
}

/// Join a community from its invite link. Returns the community id.
#[tauri::command]
pub async fn join_community(
    invite_url: String,
    state: State<'_, SharedState>,
    keystore_handle: State<'_, KeystoreHandle>,
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

#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    // Phase 5 — gate writes on lifecycle.
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    crate::services::community_lifecycle_runtime::leave_community_inner(
        state.inner(),
        &pool,
        keystore_handle.inner(),
        &community_id,
    )
    .await
}

#[tauri::command]
pub async fn update_community_info(
    community_id: String,
    name: Option<String>,
    description: Option<String>,
    icon_hash: Option<String>,
    banner_hash: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    crate::services::community_lifecycle_runtime::update_community_info_inner(
        state.inner(),
        &pool,
        community_id,
        name,
        description,
        icon_hash,
        banner_hash,
    )
    .await
}
