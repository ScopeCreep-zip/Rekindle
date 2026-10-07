//! Lost Cargo IPC commands.
//!
//! Thin wrappers — all logic lives in `services::community::files` /
//! `services::community_files_runtime`.

use tauri::State;

use crate::services::community_files_runtime::{
    download_attachment_with_dialog, reveal_downloaded_attachment_inner, send_voice_message_inner,
    upload_attachment_with_dialog, AttachmentRef,
};
use crate::state::SharedState;

/// Pick a file in a native dialog and upload it as an attachment in a
/// channel. Returns the new attachment_id (16-byte UUID, hex-encoded), or
/// `None` if the user cancelled. The file is chunked, FEK-encrypted,
/// cached locally, and announced via SMPL + gossip; downloaders find us via
/// the AttachmentCached entry written to our subkey.
#[tauri::command]
pub async fn upload_attachment(
    community_id: String,
    channel_id: String,
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Option<String>, String> {
    let pool = state.db.current()?;
    upload_attachment_with_dialog(&window, state.inner(), &pool, &community_id, &channel_id).await
}

/// Pick a save location in a native dialog and download an attachment
/// there. Returns `false` if the user cancelled.
#[tauri::command]
pub async fn download_attachment(
    community_id: String,
    channel_id: String,
    attachment_id: String,
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<bool, String> {
    let pool = state.db.current()?;
    let attachment = AttachmentRef::parse(&community_id, &channel_id, &attachment_id)?;
    download_attachment_with_dialog(&window, state.inner(), &pool, &attachment).await
}

/// A voice message's audio bytes, delivered as a binary IPC response for
/// the player to wrap in a Blob.
#[tauri::command]
pub async fn get_voice_message_audio(
    community_id: String,
    channel_id: String,
    attachment_id: String,
    state: State<'_, SharedState>,
) -> Result<tauri::ipc::Response, String> {
    let pool = state.db.current()?;
    let attachment = AttachmentRef::parse(&community_id, &channel_id, &attachment_id)?;
    let bytes = crate::services::community::files::attachment_bytes(
        state.inner(),
        &pool,
        &attachment.community,
        &attachment.channel,
        &attachment.id,
    )
    .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Show a downloaded attachment in the OS file manager.
#[tauri::command]
pub async fn reveal_downloaded_attachment(
    community_id: String,
    channel_id: String,
    attachment_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    let attachment = AttachmentRef::parse(&community_id, &channel_id, &attachment_id)?;
    reveal_downloaded_attachment_inner(&app, state.inner(), &pool, &attachment).await
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
) -> Result<String, String> {
    let pool = state.db.current()?;
    send_voice_message_inner(
        state.inner(),
        &pool,
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
