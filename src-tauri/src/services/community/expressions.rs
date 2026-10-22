//! Phase 19.i-REDO — thin facade.
//!
//! Expression uploads (emoji / sticker / soundboard) + delete + list
//! + soundboard play live in `rekindle_channel::expressions`. This
//! module constructs a `ChannelAdapter` per call and maps the crate's
//! `ExpressionView` ↔ src-tauri `ExpressionInfo`.

use crate::state::SharedState;

fn build_adapter(
    state: &SharedState,
) -> Result<crate::services::channel_adapter::ChannelAdapter, String> {
    crate::services::build_adapter(state, crate::services::channel_adapter::ChannelAdapter::new)
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
        rekindle_channel::UploadSoundboardSoundParams {
            community_id,
            name,
            bytes,
            tags,
            duration_seconds,
            volume,
            emoji,
        },
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
