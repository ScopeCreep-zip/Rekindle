//! The file transfers (plan E7.2 G12, SK2-G design §4): the Downloads
//! view's list, and Start (resume, or a first download after the native
//! save dialog), Stop (pause), Open (show in folder, never execute) and
//! Remove. Host calls (`TransferRequest`); paths never reach the webview.

use rekindle_ipc::protocol::{IpcRequest, TransferRequest};
use rekindle_types::dto::skin_run::{ShellOp, SkinRun};
use rekindle_types::dto::transfer::{TransferReveal, TransferRow, TransferTarget};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::host_call;
use crate::services::community_files_runtime::save_path;
use crate::state::SharedState;

fn transfer(request: TransferRequest) -> IpcRequest {
    IpcRequest::Transfer(request)
}

/// Every transfer, newest first.
#[tauri::command]
pub async fn transfer_list(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<TransferRow>>, CommandError> {
    host_call::call(&state, transfer(TransferRequest::TransferList)).await
}

/// Start a download: a paused or failed one resumes from its checkpoint;
/// a never-fetched attachment first asks where to save it (the Host
/// answers `ShellOp::ChooseDownloadDest`, as `download_attachment` asks).
/// `false` if the user cancelled the dialog; `true` once it finished.
#[tauri::command]
pub async fn transfer_start(
    attachment_id: String,
    target: TransferTarget,
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Done<bool>, CommandError> {
    let start = |dest| {
        transfer(TransferRequest::TransferStart {
            attachment_id: attachment_id.clone(),
            target: target.clone(),
            dest,
        })
    };
    let first: Done<Option<SkinRun>> = host_call::call(&state, start(None)).await?;
    let Some(SkinRun::Shell(ShellOp::ChooseDownloadDest { suggested_name, .. })) = first.value
    else {
        return Ok(Done {
            value: true,
            undo: first.undo,
        });
    };
    let Some(dest) = save_path(&window, &suggested_name).await? else {
        return Ok(Done {
            value: false,
            undo: None,
        });
    };
    let done: Done<Option<SkinRun>> = host_call::call(&state, start(Some(dest))).await?;
    Ok(Done {
        value: true,
        undo: done.undo,
    })
}

/// Pause a running download; its held chunks stay cached.
#[tauri::command]
pub async fn transfer_stop(
    attachment_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        transfer(TransferRequest::TransferStop { attachment_id }),
    )
    .await
}

/// Show a finished download in the OS file manager. The Host answers only
/// for a regular file at the stored path (never a link); nothing is ever
/// opened or executed (`docs/architecture/files.md`).
#[tauri::command]
pub async fn transfer_open(
    attachment_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    let reveal: Done<TransferReveal> = host_call::call(
        &state,
        transfer(TransferRequest::TransferOpen { attachment_id }),
    )
    .await?;
    app.opener()
        .reveal_item_in_dir(&reveal.value.path)
        .map_err(|e| CommandError::internal(e.to_string()))?;
    Ok(Done {
        value: (),
        undo: reveal.undo,
    })
}

/// Forget a transfer (stopping it first); `delete_file` also deletes the
/// saved file.
#[tauri::command]
pub async fn transfer_remove(
    attachment_id: String,
    delete_file: bool,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        transfer(TransferRequest::TransferRemove {
            attachment_id,
            delete_file,
        }),
    )
    .await
}
