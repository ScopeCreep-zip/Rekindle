//! Phase 23.C — Lost Cargo file-handler runtime orchestration lifted
//! from `commands/community/files.rs`: voice-message upload, and the
//! upload / download / reveal flows that use native dialogs. File paths
//! are chosen in native dialogs and never cross the IPC boundary.

use std::path::PathBuf;

use rekindle_types::key_format;
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;

use crate::state::SharedState;
use rekindle_db::Db;

pub async fn send_voice_message_inner(
    state: &SharedState,
    pool: &Db,
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

/// Ids a file command takes, validated at the IPC boundary.
pub struct AttachmentRef {
    pub community: String,
    pub channel: String,
    pub id: String,
}

impl AttachmentRef {
    pub fn parse(
        community_id: &str,
        channel_id: &str,
        attachment_id: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            community: validate_community(community_id)?,
            channel: validate_channel(channel_id)?,
            id: key_format::hex16_id(attachment_id)
                .map_err(|e| format!("invalid attachment id: {e}"))?
                .into_string(),
        })
    }
}

fn validate_community(community_id: &str) -> Result<String, String> {
    key_format::record_key(community_id)
        .map(key_format::RecordKeyStr::into_string)
        .map_err(|e| format!("invalid community id: {e}"))
}

fn validate_channel(channel_id: &str) -> Result<String, String> {
    key_format::hex16_id(channel_id)
        .map(key_format::Hex16Id::into_string)
        .map_err(|e| format!("invalid channel id: {e}"))
}

/// Run a native file dialog and wait for its answer without blocking an
/// async worker (the dialog callback fires on the UI thread).
async fn dialog_path(
    show: impl FnOnce(Box<dyn FnOnce(Option<FilePath>) + Send>),
) -> Result<Option<PathBuf>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    show(Box::new(move |picked| {
        let _ = tx.send(picked);
    }));
    match rx
        .await
        .map_err(|_| "file dialog closed unexpectedly".to_string())?
    {
        Some(path) => path.into_path().map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Ask for a file and upload it to a channel. `None` if the user cancelled.
pub async fn upload_attachment_with_dialog(
    window: &tauri::WebviewWindow,
    state: &SharedState,
    pool: &Db,
    community_id: &str,
    channel_id: &str,
) -> Result<Option<String>, String> {
    let community_id = validate_community(community_id)?;
    let channel_id = validate_channel(channel_id)?;
    let builder = window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Attach a file");
    let Some(path) = dialog_path(|done| builder.pick_file(done)).await? else {
        return Ok(None);
    };
    crate::services::community::files::upload_file(state, pool, &community_id, &channel_id, &path)
        .await
        .map(Some)
}

/// Ask where to save an attachment, then download it there. `false` if the
/// user cancelled. The suggested name is the uploader's filename, sanitized.
pub async fn download_attachment_with_dialog(
    window: &tauri::WebviewWindow,
    state: &SharedState,
    pool: &Db,
    attachment: &AttachmentRef,
) -> Result<bool, String> {
    let record = crate::services::community::files::attachment_record(
        state,
        pool,
        &attachment.channel,
        &attachment.id,
    )
    .await?
    .ok_or("attachment not found in this channel")?;
    let builder = window
        .dialog()
        .file()
        .set_parent(window)
        .set_file_name(rekindle_files::sanitize_filename(&record.filename));
    let Some(path) = dialog_path(|done| builder.save_file(done)).await? else {
        return Ok(false);
    };
    crate::services::community::files::download_attachment(
        state,
        pool,
        &attachment.community,
        &attachment.channel,
        &attachment.id,
        &path,
    )
    .await?;
    Ok(true)
}

/// Show a downloaded (or self-uploaded) attachment in the OS file manager.
pub async fn reveal_downloaded_attachment_inner(
    app: &tauri::AppHandle,
    state: &SharedState,
    pool: &Db,
    attachment: &AttachmentRef,
) -> Result<(), String> {
    let record = crate::services::community::files::attachment_record(
        state,
        pool,
        &attachment.channel,
        &attachment.id,
    )
    .await?
    .ok_or("attachment not found in this channel")?;
    let path = PathBuf::from(
        record
            .local_path
            .ok_or("attachment has not been downloaded")?,
    );
    if !path.exists() {
        return Err("the downloaded file has been moved or deleted".into());
    }
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())
}
