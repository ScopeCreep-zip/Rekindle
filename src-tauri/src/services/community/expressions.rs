//! Phase 19.i-REDO — thin facade.
//!
//! Expression uploads (emoji / sticker / soundboard) + delete + list
//! + soundboard play live in `rekindle_channel::expressions`. This
//! module constructs a `ChannelAdapter` per call and maps the crate's
//! `ExpressionView` ↔ src-tauri `ExpressionInfo`.

use std::sync::Arc;

use rekindle_types::expression::SoundboardMeta;
use tauri::Manager;

use crate::state::SharedState;

#[derive(Debug, Clone)]
pub struct ExpressionInfo {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    pub inline_data_base64: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    pub sound_meta: Option<SoundboardMeta>,
    pub creator_pseudonym: Option<String>,
    pub created_at: Option<u64>,
    pub available_to_peers: bool,
}

fn build_adapter(
    state: &SharedState,
) -> Result<crate::services::channel_adapter::ChannelAdapter, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<crate::db::DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    Ok(crate::services::channel_adapter::ChannelAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    ))
}

fn view_to_info(view: rekindle_channel::deps::ExpressionView) -> ExpressionInfo {
    ExpressionInfo {
        expression_id: view.expression_id,
        name: view.name,
        kind: view.kind,
        content_hash: view.content_hash,
        inline_data_base64: view.inline_data_base64,
        media_type: view.media_type,
        animated: view.animated,
        tags: view.tags,
        sound_meta: view.sound_meta,
        creator_pseudonym: view.creator_pseudonym,
        created_at: view.created_at,
        available_to_peers: view.available_to_peers,
    }
}

pub async fn upload_emoji(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    animated: bool,
) -> Result<String, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::upload_emoji(&adapter, community_id, name, bytes, animated)
        .await
        .map_err(|e| e.to_string())
}

pub async fn upload_sticker(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    animated: bool,
    tags: Vec<String>,
) -> Result<String, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::upload_sticker(&adapter, community_id, name, bytes, animated, tags)
        .await
        .map_err(|e| e.to_string())
}

pub async fn upload_soundboard_sound(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    tags: Vec<String>,
    duration_seconds: f32,
    volume: f32,
    emoji: Option<String>,
) -> Result<String, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::upload_soundboard_sound(
        &adapter,
        community_id,
        name,
        bytes,
        tags,
        duration_seconds,
        volume,
        emoji,
    )
    .await
    .map_err(|e| e.to_string())
}

pub fn play_soundboard(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    expression_id_hex: &str,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::play_soundboard(&adapter, community_id, channel_id, expression_id_hex)
        .map_err(|e| e.to_string())
}

pub async fn delete_expression(
    state: &SharedState,
    community_id: &str,
    expression_id_hex: &str,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::delete_expression(&adapter, community_id, expression_id_hex)
        .await
        .map_err(|e| e.to_string())
}

pub fn list_expressions(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<ExpressionInfo>, String> {
    let adapter = build_adapter(state)?;
    let views = rekindle_channel::list_expressions(&adapter, community_id)
        .map_err(|e| e.to_string())?;
    Ok(views.into_iter().map(view_to_info).collect())
}
