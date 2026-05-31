use tauri::State;

use crate::db::DbPool;
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

#[tauri::command]
pub async fn create_community(
    _app: tauri::AppHandle,
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<String, String> {
    crate::services::community_lifecycle_runtime::create_community_inner(
        state.inner(),
        pool.inner(),
        keystore_handle.inner(),
        name,
    )
    .await
}

#[tauri::command]
pub async fn join_community(
    community_id: String,
    invite_code: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    let _g = rekindle_lifecycle::TransportGuard::write(&state.lifecycle)
        .map_err(|e| e.to_string())?;
    crate::services::community_lifecycle_runtime::join_community_inner(
        state.inner(),
        pool.inner(),
        keystore_handle.inner(),
        community_id,
        invite_code,
    )
    .await
}

#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    // Phase 5 — gate writes on lifecycle.
    let _g = rekindle_lifecycle::TransportGuard::write(&state.lifecycle)
        .map_err(|e| e.to_string())?;
    crate::services::community_lifecycle_runtime::leave_community_inner(
        state.inner(),
        pool.inner(),
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
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    crate::services::community_lifecycle_runtime::update_community_info_inner(
        state.inner(),
        pool.inner(),
        community_id,
        name,
        description,
        icon_hash,
        banner_hash,
    )
    .await
}
