//! The attachment dialogs (plan E3.1 E3d): the shell picks the file to
//! upload or the place to save in a native dialog, and the Host does the
//! rest (`PinRequest`). The reveal shows a downloaded file in the OS file
//! manager, where the Host's record says it was saved.

use std::path::PathBuf;

use rekindle_files::AttachmentRecordJson;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, PinRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;

use crate::state::SharedState;

/// A pin-domain request as an `IpcRequest`.
pub fn pins(request: PinRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Pins(request))
}

/// Run a native file dialog and wait for its answer without blocking an
/// async worker (the dialog callback fires on the UI thread).
async fn dialog_path(
    show: impl FnOnce(Box<dyn FnOnce(Option<FilePath>) + Send>),
) -> Result<Option<PathBuf>, CommandError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    show(Box::new(move |picked| {
        let _ = tx.send(picked);
    }));
    match rx
        .await
        .map_err(|_| CommandError::internal("file dialog closed unexpectedly"))?
    {
        Some(path) => path
            .into_path()
            .map(Some)
            .map_err(|e| CommandError::internal(e.to_string())),
        None => Ok(None),
    }
}

fn path_string(path: PathBuf) -> Result<String, CommandError> {
    path.into_os_string()
        .into_string()
        .map_err(|_| CommandError::internal("the picked path is not UTF-8"))
}

/// The Host's record of an attachment, or a 404.
async fn record(
    state: &SharedState,
    community: &str,
    channel: &str,
    attachment: &str,
) -> Result<AttachmentRecordJson, CommandError> {
    let found: Done<Option<AttachmentRecordJson>> = crate::host_call::call(
        state,
        pins(PinRequest::AttachmentInfo {
            community: community.to_owned(),
            channel_id: channel.to_owned(),
            attachment_id: attachment.to_owned(),
        }),
    )
    .await?;
    found.value.ok_or(CommandError::Failed {
        code: 404,
        message: "attachment not found in this channel".into(),
        remediation: None,
    })
}

/// Ask for a file and upload it to a channel: the attachment id, or `None`
/// if the user cancelled.
pub async fn upload_with_dialog(
    window: &tauri::WebviewWindow,
    state: &SharedState,
    community: String,
    channel: String,
) -> Result<Done<Option<String>>, CommandError> {
    let builder = window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Attach a file");
    let Some(path) = dialog_path(|done| builder.pick_file(done)).await? else {
        return Ok(Done {
            value: None,
            undo: None,
        });
    };
    let done: Done<String> = crate::host_call::call(
        state,
        pins(PinRequest::AttachmentUpload {
            community,
            channel_id: channel,
            path: path_string(path)?,
        }),
    )
    .await?;
    Ok(Done {
        value: Some(done.value),
        undo: done.undo,
    })
}

/// Ask where to save an attachment, then have the Host download it there:
/// `false` if the user cancelled. The suggested name is the uploader's
/// filename, sanitized.
pub async fn download_with_dialog(
    window: &tauri::WebviewWindow,
    state: &SharedState,
    community: String,
    channel: String,
    attachment: String,
) -> Result<Done<bool>, CommandError> {
    let found = record(state, &community, &channel, &attachment).await?;
    let builder = window
        .dialog()
        .file()
        .set_parent(window)
        .set_file_name(rekindle_files::sanitize_filename(&found.filename));
    let Some(path) = dialog_path(|done| builder.save_file(done)).await? else {
        return Ok(Done {
            value: false,
            undo: None,
        });
    };
    let done: Done<()> = crate::host_call::call(
        state,
        pins(PinRequest::AttachmentDownload {
            community,
            channel_id: channel,
            attachment_id: attachment,
            path: path_string(path)?,
        }),
    )
    .await?;
    Ok(Done {
        value: true,
        undo: done.undo,
    })
}

/// Show a downloaded (or self-uploaded) attachment in the OS file manager.
pub async fn reveal(
    app: &tauri::AppHandle,
    state: &SharedState,
    community: &str,
    channel: &str,
    attachment: &str,
) -> Result<Done<()>, CommandError> {
    let found = record(state, community, channel, attachment).await?;
    let failed = |message: &str| CommandError::Failed {
        code: 404,
        message: message.into(),
        remediation: None,
    };
    let path = PathBuf::from(
        found
            .local_path
            .ok_or_else(|| failed("attachment has not been downloaded"))?,
    );
    if !path.exists() {
        return Err(failed("the downloaded file has been moved or deleted"));
    }
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| CommandError::internal(e.to_string()))?;
    Ok(Done {
        value: (),
        undo: None,
    })
}
