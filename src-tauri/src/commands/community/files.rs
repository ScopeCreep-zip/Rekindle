//! Lost Cargo IPC commands (plan E3.1 E3d): the shell's dialogs, then the
//! Host (`PinRequest`).

use rekindle_ipc::protocol::PinRequest;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::services::community_files_runtime::{
    download_with_dialog, pins, reveal, upload_with_dialog,
};
use crate::state::SharedState;

/// Pick a file in a native dialog and have the Host upload it as an
/// attachment in a channel: the new attachment id (16 bytes, hex), or
/// `None` if the user cancelled.
#[tauri::command]
pub async fn upload_attachment(
    community_id: String,
    channel_id: String,
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Done<Option<String>>, CommandError> {
    upload_with_dialog(&window, state.inner(), community_id, channel_id).await
}

/// Pick a save location in a native dialog and have the Host download an
/// attachment there: `false` if the user cancelled.
#[tauri::command]
pub async fn download_attachment(
    community_id: String,
    channel_id: String,
    attachment_id: String,
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Done<bool>, CommandError> {
    download_with_dialog(
        &window,
        state.inner(),
        community_id,
        channel_id,
        attachment_id,
    )
    .await
}

/// A voice message's audio bytes, delivered as a binary IPC response for
/// the player to wrap in a Blob.
#[tauri::command]
pub async fn get_voice_message_audio(
    community_id: String,
    channel_id: String,
    attachment_id: String,
    state: State<'_, SharedState>,
) -> Result<tauri::ipc::Response, CommandError> {
    let done: Done<Vec<u8>> = crate::host_call::call(
        &state,
        pins(PinRequest::VoiceMessageAudio {
            community: community_id,
            channel_id,
            attachment_id,
        }),
    )
    .await?;
    Ok(tauri::ipc::Response::new(done.value))
}

/// Show a downloaded attachment in the OS file manager.
#[tauri::command]
pub async fn reveal_downloaded_attachment(
    community_id: String,
    channel_id: String,
    attachment_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    reveal(
        &app,
        state.inner(),
        &community_id,
        &channel_id,
        &attachment_id,
    )
    .await
}

/// Send a voice message (architecture §16.4): its attachment id.
#[tauri::command]
pub async fn send_voice_message(
    community_id: String,
    channel_id: String,
    opus_bytes_b64: String,
    duration_ms: u32,
    waveform_b64: String,
    state: State<'_, SharedState>,
) -> Result<Done<String>, CommandError> {
    use base64::Engine as _;
    let decode = |b64: &str, what: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| CommandError::Failed {
                code: 400,
                message: format!("invalid {what} base64: {e}"),
                remediation: None,
            })
    };
    crate::host_call::call(
        &state,
        pins(PinRequest::VoiceMessageSend {
            community: community_id,
            channel_id,
            opus: decode(&opus_bytes_b64, "opus")?,
            duration_ms,
            waveform: decode(&waveform_b64, "waveform")?,
        }),
    )
    .await
}

/// Pin or unpin an attachment (admin-only; the Host's, plan E3.1 E3b).
#[tauri::command]
pub async fn pin_attachment(
    community_id: String,
    attachment_id: String,
    pinned: bool,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    crate::host_call::call(
        &state,
        pins(PinRequest::AttachmentPin {
            community: community_id,
            attachment_id,
            pinned,
        }),
    )
    .await
}
