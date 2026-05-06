use tauri::State;

use crate::state::SharedState;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionInfoDto {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    pub inline_data_base64: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    /// Architecture §18.3 — present only on `kind == "soundboard"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_meta: Option<rekindle_types::expression::SoundboardMeta>,
    /// Architecture §18.1 line 2455 — uploader's per-community pseudonym (hex).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_pseudonym: Option<String>,
    /// Architecture §18.1 line 2456 — wall-clock seconds at upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Architecture §18.1 line 2459 — gates `USE_EXTERNAL_EMOJIS`.
    pub available_to_peers: bool,
}

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
    let expressions = crate::services::community::list_expressions(state.inner(), &community_id)?;
    Ok(expressions
        .into_iter()
        .map(|expression| ExpressionInfoDto {
            expression_id: expression.expression_id,
            name: expression.name,
            kind: expression.kind,
            content_hash: expression.content_hash,
            inline_data_base64: expression.inline_data_base64,
            media_type: expression.media_type,
            animated: expression.animated,
            tags: expression.tags,
            sound_meta: expression.sound_meta,
            creator_pseudonym: expression.creator_pseudonym,
            created_at: expression.created_at,
            available_to_peers: expression.available_to_peers,
        })
        .collect())
}
