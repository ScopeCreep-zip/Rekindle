//! Tauri command for sender-side link preview generation
//! (architecture §28.8). The frontend detects URLs in composition,
//! calls this once per URL, and the backend handles fetching and
//! gossip broadcast.

use rekindle_types::link_preview::LinkPreview;
use tauri::State;

use crate::db::DbPool;
use crate::services::community::link_previews;
use crate::services::community_link_previews_runtime::{
    get_link_previews_enabled_inner, set_link_previews_enabled_inner,
};
use crate::state::SharedState;

#[tauri::command]
pub async fn fetch_link_preview(
    community_id: String,
    channel_id: String,
    message_id: String,
    url: String,
    state: State<'_, SharedState>,
) -> Result<LinkPreview, String> {
    link_previews::fetch_and_broadcast(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &url,
    )
    .await
}

/// Architecture §28.8 line 3220 — IP-privacy toggle for link preview
/// generation.
#[tauri::command]
pub async fn set_link_previews_enabled(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    enabled: bool,
) -> Result<(), String> {
    set_link_previews_enabled_inner(state.inner(), pool.inner(), enabled).await
}

#[tauri::command]
pub async fn get_link_previews_enabled(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<bool, String> {
    get_link_previews_enabled_inner(state.inner(), pool.inner()).await
}
