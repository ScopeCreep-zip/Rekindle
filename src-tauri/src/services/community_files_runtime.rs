//! Phase 23.C — Lost Cargo file-handler runtime orchestration lifted
//! from `commands/community/files.rs`. Hosts `send_voice_message_inner`
//! (base64 decode of opus + waveform, then crate delegate) and
//! `download_attachment_inner` (path conversion + delegate + emit).

use std::path::PathBuf;

use crate::db::DbPool;
use crate::state::SharedState;

pub async fn send_voice_message_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    opus_bytes_b64: &str,
    duration_ms: u32,
    waveform_b64: &str,
) -> Result<String, String> {
    use base64::Engine as _;
    let opus_bytes = base64::engine::general_purpose::STANDARD
        .decode(opus_bytes_b64)
        .map_err(|e| format!("invalid opus_bytes base64: {e}"))?;
    let waveform = base64::engine::general_purpose::STANDARD
        .decode(waveform_b64)
        .map_err(|e| format!("invalid waveform base64: {e}"))?;
    crate::services::community::files::send_voice_message_bytes(
        state,
        pool,
        community_id,
        channel_id,
        opus_bytes,
        duration_ms,
        waveform,
    )
    .await
}

pub async fn download_attachment_inner(
    state: &SharedState,
    pool: &DbPool,
    app: &tauri::AppHandle,
    community_id: &str,
    channel_id: &str,
    attachment_id: &str,
    save_path: String,
) -> Result<(), String> {
    let save_path = PathBuf::from(save_path);
    crate::services::community::files::download_attachment(
        state,
        pool,
        community_id,
        channel_id,
        attachment_id,
        &save_path,
    )
    .await?;
    crate::services::community::files::emit_attachment_complete(
        app,
        community_id,
        channel_id,
        attachment_id,
        &save_path,
    );
    Ok(())
}
