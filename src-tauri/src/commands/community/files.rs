//! Lost Cargo IPC commands.
//!
//! Thin wrappers — all logic lives in `services::community::files`.

use std::path::PathBuf;

use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;

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

/// Download an attachment by hex id to `save_path`. Walks SMPL subkeys
/// to find sources, requests missing chunks via `app_call`, verifies
/// + reassembles, advertises full possession back to the swarm. Emits
/// a `community-event` `AttachmentDownloaded` on success.
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
    let save_path = PathBuf::from(save_path);
    crate::services::community::files::download_attachment(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        &attachment_id,
        &save_path,
    )
    .await?;
    crate::services::community::files::emit_attachment_complete(
        &app,
        &community_id,
        &channel_id,
        &attachment_id,
        &save_path,
    );
    Ok(())
}

/// Send a voice message (architecture §16.4) — a `ChannelEntry::Message`
/// with `flags |= VOICE_MESSAGE` carrying an `audio/ogg` Lost Cargo
/// attachment plus waveform + duration metadata in the body.
///
/// The frontend records via MediaRecorder, base64-encodes the bytes, and
/// passes them along with the duration and waveform peaks (≤256 u8s).
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
    use base64::Engine as _;
    let opus_bytes = base64::engine::general_purpose::STANDARD
        .decode(&opus_bytes_b64)
        .map_err(|e| format!("invalid opus_bytes base64: {e}"))?;
    let waveform = base64::engine::general_purpose::STANDARD
        .decode(&waveform_b64)
        .map_err(|e| format!("invalid waveform base64: {e}"))?;
    crate::services::community::files::send_voice_message_bytes(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        opus_bytes,
        duration_ms,
        waveform,
    )
    .await
}

/// Pin or unpin an attachment (admin-only). Pinned attachments are
/// exempt from local LRU eviction in every member's cache. Writes a
/// `GovernanceEntry::AttachmentPinned` — the merged state propagates
/// to all peers via the existing governance sync paths.
#[tauri::command]
pub async fn pin_attachment(
    community_id: String,
    attachment_id: String,
    pinned: bool,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let _ = state_helpers::current_owner_key(state.inner())?;
    crate::services::community::files::set_attachment_pinned(
        state.inner(),
        &community_id,
        &attachment_id,
        pinned,
    )
    .await
}
