use tauri::State;

use crate::db::DbPool;
use crate::services::community_channel_runtime::{
    create_category_inner, delete_category_inner, move_channel_inner, rename_category_inner,
    reorder_categories_inner, reorder_channels_inner, set_channel_forum_tags_inner,
    set_channel_topic_inner,
};
use crate::state::SharedState;

/// Create a new community channel.
///
/// `parent_voice_channel_id` (architecture §10.8 — text-in-voice):
/// when set, the new channel is the text-in-voice companion of that
/// voice channel; the frontend hides it unless the local member is
/// currently joined to the parent voice channel.
///
/// Phase 8 — `idempotency_key` (UUID v7) dedupes click-spam. Channel
/// creation is **especially** sensitive: each call allocates a fresh
/// DHT record (network round-trip + storage cost) AND writes a
/// `ChannelCreated` governance entry with a fresh 16-byte id, so two
/// rapid clicks would create two distinct channels with the same
/// name. Uses `idempotency_string` (separate cache from
/// `idempotency` because the return type is `Result<String,String>`).
#[tauri::command]
pub async fn create_channel(
    community_id: String,
    name: String,
    channel_type: String,
    category_id: Option<String>,
    parent_voice_channel_id: Option<String>,
    idempotency_key: uuid::Uuid,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    // Phase 5 — gate writes on lifecycle.
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    let state_clone = state.inner().clone();
    let pool_clone = pool.inner().clone();
    state
        .idempotency_string
        .wrap(idempotency_key, || async move {
            crate::services::community_channel_runtime::create_channel_inner(
                state_clone,
                pool_clone,
                community_id,
                name,
                channel_type,
                category_id,
                parent_voice_channel_id,
            )
            .await
        })
        .await
}

#[tauri::command]
pub async fn create_category(
    community_id: String,
    name: String,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    create_category_inner(state.inner(), community_id, name).await
}

#[tauri::command]
pub async fn delete_category(
    community_id: String,
    category_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    delete_category_inner(state.inner(), community_id, category_id).await
}

#[tauri::command]
pub async fn rename_category(
    community_id: String,
    category_id: String,
    new_name: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    rename_category_inner(state.inner(), community_id, category_id, new_name).await
}

#[tauri::command]
pub async fn move_channel(
    community_id: String,
    channel_id: String,
    category_id: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    move_channel_inner(state.inner(), community_id, channel_id, category_id).await
}

#[tauri::command]
pub async fn reorder_categories(
    community_id: String,
    category_ids: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    reorder_categories_inner(state.inner(), community_id, category_ids).await
}

#[tauri::command]
pub async fn set_channel_topic(
    community_id: String,
    channel_id: String,
    topic: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    set_channel_topic_inner(state.inner(), community_id, channel_id, topic).await
}

#[tauri::command]
pub async fn set_channel_forum_tags(
    community_id: String,
    channel_id: String,
    forum_tags: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    set_channel_forum_tags_inner(state.inner(), community_id, channel_id, forum_tags).await
}

#[tauri::command]
pub async fn reorder_channels(
    community_id: String,
    channel_ids: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    reorder_channels_inner(state.inner(), community_id, channel_ids).await
}
