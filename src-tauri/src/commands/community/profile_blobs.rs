//! Architecture §32 Phase 5 Week 15 — per-community avatar + banner.
//!
//! Tauri command surface only. The actual WebP compression pipeline +
//! content-addressed cache lives in
//! `services::community_profile_blobs_runtime`. Returns the BLAKE3 hex
//! hash on upload; reads return a `data:image/webp;base64,...` URL.

use tauri::State;

use crate::services::community_profile_blobs_runtime::{
    read_data_url, store_blob, AVATAR_MAX_DIM, BANNER_MAX_H, BANNER_MAX_W, MAX_RAW_AVATAR_BYTES,
    MAX_RAW_BANNER_BYTES,
};
use crate::state::SharedState;

#[tauri::command]
pub async fn set_community_avatar(
    state: State<'_, SharedState>,
    community_id: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    store_blob(
        state.inner(),
        &community_id,
        bytes,
        MAX_RAW_AVATAR_BYTES,
        AVATAR_MAX_DIM,
        AVATAR_MAX_DIM,
    )
    .await
}

#[tauri::command]
pub async fn set_community_banner(
    state: State<'_, SharedState>,
    community_id: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    store_blob(
        state.inner(),
        &community_id,
        bytes,
        MAX_RAW_BANNER_BYTES,
        BANNER_MAX_W,
        BANNER_MAX_H,
    )
    .await
}

#[tauri::command]
pub async fn get_community_avatar_data_url(
    state: State<'_, SharedState>,
    community_id: String,
    hash: String,
) -> Result<Option<String>, String> {
    read_data_url(state.inner(), &community_id, &hash)
}
