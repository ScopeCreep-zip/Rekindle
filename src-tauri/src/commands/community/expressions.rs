use tauri::State;

use crate::services::community_views_runtime::ExpressionInfoDto;
use crate::state::SharedState;

#[tauri::command]
pub async fn upload_emoji(
    state: State<'_, SharedState>,
    community_id: String,
    name: String,
    bytes: Vec<u8>,
    animated: bool,
) -> Result<String, String> {
    crate::services::community::upload_emoji(state.inner(), &community_id, &name, bytes, animated)
        .await
}

#[tauri::command]
pub async fn upload_sticker(
    state: State<'_, SharedState>,
    community_id: String,
    name: String,
    bytes: Vec<u8>,
    animated: bool,
    tags: Option<Vec<String>>,
) -> Result<String, String> {
    crate::services::community::upload_sticker(
        state.inner(),
        &community_id,
        &name,
        bytes,
        animated,
        tags.unwrap_or_default(),
    )
    .await
}

#[tauri::command]
pub async fn upload_soundboard_sound(
    state: State<'_, SharedState>,
    community_id: String,
    name: String,
    bytes: Vec<u8>,
    tags: Option<Vec<String>>,
    duration_seconds: f32,
    volume: f32,
    emoji: Option<String>,
) -> Result<String, String> {
    crate::services::community::upload_soundboard_sound(
        state.inner(),
        &community_id,
        &name,
        bytes,
        tags.unwrap_or_default(),
        duration_seconds,
        volume,
        emoji,
    )
    .await
}

#[tauri::command]
pub async fn play_soundboard(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    expression_id: String,
) -> Result<(), String> {
    crate::services::community::play_soundboard(
        state.inner(),
        &community_id,
        &channel_id,
        &expression_id,
    )
}

#[tauri::command]
pub async fn delete_emoji(
    state: State<'_, SharedState>,
    community_id: String,
    expression_id: String,
) -> Result<(), String> {
    crate::services::community::delete_expression(state.inner(), &community_id, &expression_id)
        .await
}

#[tauri::command]
pub async fn list_expressions(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Vec<ExpressionInfoDto>, String> {
    crate::services::community_views_runtime::list_expressions_inner(
        state.inner(),
        &community_id,
    )
}
