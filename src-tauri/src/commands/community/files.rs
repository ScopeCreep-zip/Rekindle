//! Lost Cargo IPC commands.
//!
//! Thin wrappers — all logic lives in `services::community::files` /
//! `services::community_files_runtime`.

use std::path::PathBuf;

use tauri::State;

use crate::db::DbPool;
use crate::services::community_files_runtime::{
    download_attachment_inner, send_voice_message_inner,
};
use crate::state::SharedState;

/// Upload a file as an attachment in a channel. Returns the new
/// attachment_id (16-byte UUID, hex-encoded). The file is chunked,
/// FEK-encrypted, cached locally, and announced via SMPL +
/// gossip; downloaders find us via the AttachmentCached entry written
/// to our subkey.
#[tauri::command]
pub async fn upload_attachment(
    community_id: String,
    channel_id: String,
    file_path: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let path = PathBuf::from(file_path);
    crate::services::community::files::upload_file(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        &path,
    )
    .await
}

/// Download an attachment by hex id to `save_path`.
#[tauri::command]
pub async fn download_attachment(
    community_id: String,
    channel_id: String,
    attachment_id: String,
    save_path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    download_attachment_inner(
        state.inner(),
        pool.inner(),
        &app,
        &community_id,
        &channel_id,
        &attachment_id,
        save_path,
    )
    .await
}

/// Send a voice message (architecture §16.4).
#[tauri::command]
pub async fn send_voice_message(
    community_id: String,
    channel_id: String,
    opus_bytes_b64: String,
    duration_ms: u32,
    waveform_b64: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    send_voice_message_inner(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        &opus_bytes_b64,
        duration_ms,
        &waveform_b64,
    )
    .await
}

/// Pin or unpin an attachment (admin-only).
#[tauri::command]
pub async fn pin_attachment(
    community_id: String,
    attachment_id: String,
    pinned: bool,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::services::community::files::set_attachment_pinned(
        state.inner(),
        &community_id,
        &attachment_id,
        pinned,
    )
    .await
}
