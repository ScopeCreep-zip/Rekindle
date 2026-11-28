//! The skins (plan E7.7.2): what a window applies (the windows re-apply
//! on `SystemEvent::SkinChanged`) and Settings → Appearance's library,
//! choice, trial and login skin. Host calls, one request each.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::skin::RegionFit;
use rekindle_types::dto::skin_action::SkinAction;
use rekindle_types::dto::skin_draft::{SkinDraft, SkinDraftRegion, SkinDraftReport};
use rekindle_types::dto::skin_library::{SkinApplied, SkinApplyFailure, SkinEntry, SkinLibrary};
use rekindle_types::dto::skin_run::SkinSelection;
use rekindle_types::dto::{CommandError, Done};
use tauri::{State, WebviewWindow};

use crate::host_call;
use crate::state::SharedState;

/// What this window applies: `id`, or with `None` the skin this device
/// shows now (`IpcRequest::SkinGet`).
#[tauri::command]
pub async fn get_skin(
    id: Option<String>,
    window: WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Done<SkinApplied>, CommandError> {
    if id.is_none() && crate::windows::frame_of_label(window.label()).is_some() {
        return crate::windows::presented_skin(&window)
            .await
            .map(|value| Done { value, undo: None })
            .ok_or_else(|| CommandError::Failed {
                code: 503,
                message: "The window is waiting for its skin allocation".to_owned(),
                remediation: None,
            });
    }
    host_call::call(&state, IpcRequest::SkinGet { id }).await
}

/// The skins (Settings → Appearance): the built-ins, the identity's
/// library, its choice, the device login skin and a trial in progress.
#[tauri::command]
pub async fn list_skins(state: State<'_, SharedState>) -> Result<Done<SkinLibrary>, CommandError> {
    host_call::call(&state, IpcRequest::SkinList).await
}

/// Import a `.rkskin` package; the Host validates it in the helper and
/// reviews the import (trust-changing).
#[tauri::command]
pub async fn import_skin(
    bytes: Vec<u8>,
    state: State<'_, SharedState>,
) -> Result<Done<SkinEntry>, CommandError> {
    host_call::call(&state, IpcRequest::SkinImport { bytes }).await
}

/// Remove a library skin (not the one in use or on trial).
#[tauri::command]
pub async fn remove_skin(
    id: String,
    state: State<'_, SharedState>,
) -> Result<Done<SkinLibrary>, CommandError> {
    host_call::call(&state, IpcRequest::SkinRemove { id }).await
}

/// Choose the identity's skin: a library skin goes on trial and the Host
/// reverts it unless it is kept (`keep_skin`).
#[tauri::command]
pub async fn select_skin(
    id: String,
    state: State<'_, SharedState>,
) -> Result<Done<SkinLibrary>, CommandError> {
    host_call::call(&state, IpcRequest::SkinSelect { id }).await
}

/// Settle the "Keep this skin?" trial of `id`.
#[tauri::command]
pub async fn keep_skin(
    id: String,
    keep: bool,
    state: State<'_, SharedState>,
) -> Result<Done<SkinLibrary>, CommandError> {
    host_call::call(&state, IpcRequest::SkinKeep { id, keep }).await
}

/// The skin this device's login screen shows, before unlock.
#[tauri::command]
pub async fn set_login_skin(
    id: String,
    state: State<'_, SharedState>,
) -> Result<Done<SkinLibrary>, CommandError> {
    host_call::call(&state, IpcRequest::SkinSetLogin { id }).await
}

/// The skin editor (plan E7.7.5): the Host's report on a draft.
#[tauri::command]
pub async fn validate_skin_draft(
    draft: SkinDraft,
    state: State<'_, SharedState>,
) -> Result<Done<SkinDraftReport>, CommandError> {
    host_call::call(&state, IpcRequest::SkinValidateDraft { draft }).await
}

/// "Start from the current skin": `id`'s draft, or the skin shown now.
#[tauri::command]
pub async fn skin_draft_start(
    id: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<SkinDraft>, CommandError> {
    host_call::call(&state, IpcRequest::SkinDraftStart { id }).await
}

/// Ask where to save the draft, then have the Host validate and write it:
/// the package address, or `None` if the user cancelled.
#[tauri::command]
pub async fn export_skin_draft(
    draft: SkinDraft,
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Done<Option<String>>, CommandError> {
    let Some(path) =
        crate::services::community_files_runtime::save_path(&window, "skin.rkskin").await?
    else {
        return Ok(Done {
            value: None,
            undo: None,
        });
    };
    let done: Done<String> =
        host_call::call(&state, IpcRequest::SkinExportDraft { draft, path }).await?;
    Ok(Done {
        value: Some(done.value),
        undo: done.undo,
    })
}

/// Region import (plan E7.7.5): ask for a PNG or GIF, then have the Host
/// run the draft with it through the validator. The draft with the region,
/// or `None` if the user cancelled; a bad image or nine-slice is refused
/// by name and the draft is unchanged.
#[tauri::command]
pub async fn import_skin_region(
    draft: SkinDraft,
    region: String,
    fit: RegionFit,
    text_backing: Option<String>,
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Done<Option<SkinDraft>>, CommandError> {
    let picked = crate::services::community_files_runtime::open_path(
        &window,
        "Import a region image",
        "PNG or GIF",
        &["png", "gif"],
    )
    .await?;
    let Some(path) = picked else {
        return Ok(Done {
            value: None,
            undo: None,
        });
    };
    let mut next = draft;
    next.regions.insert(
        region,
        SkinDraftRegion {
            path,
            fit,
            text_backing,
        },
    );
    let checked: Done<String> = host_call::call(
        &state,
        IpcRequest::SkinDraftCheck {
            draft: next.clone(),
        },
    )
    .await?;
    Ok(Done {
        value: Some(next),
        undo: checked.undo,
    })
}

/// Ask where to save the token file's JSON Schema (for DTCG tools), then
/// have the Host write it: `true` once written, `false` if cancelled.
#[tauri::command]
pub async fn export_skin_schema(
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
) -> Result<Done<bool>, CommandError> {
    let Some(path) =
        crate::services::community_files_runtime::save_path(&window, "rekindle-skin.schema.json")
            .await?
    else {
        return Ok(Done {
            value: false,
            undo: None,
        });
    };
    let done: Done<()> = host_call::call(&state, IpcRequest::SkinExportSchema { path }).await?;
    Ok(Done {
        value: true,
        undo: done.undo,
    })
}

/// A skin node's trigger (plan SK2-A4): the Host's table runs it, a
/// feature as its own request through the intent gate; any other answers
/// `dto::skin_run::SkinRun` for the window's shell. The window is the
/// calling one, by label (lead decision 48): a webview never names
/// another window's selection.
#[tauri::command]
pub async fn run_skin_action(
    state: State<'_, SharedState>,
    window: tauri::WebviewWindow,
    node: String,
    action: SkinAction,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::RunSkinAction {
            window: window.label().to_owned(),
            node,
            action,
        },
    )
    .await
}

/// What the calling skin window has selected, kept by the Host by window
/// label (plan SK2-A4, lead decision 48).
#[tauri::command]
pub async fn set_skin_selection(
    state: State<'_, SharedState>,
    window: tauri::WebviewWindow,
    selection: Option<SkinSelection>,
) -> Result<Done<()>, CommandError> {
    let window = window.label().to_owned();
    host_call::call(&state, IpcRequest::SkinSelectionSet { window, selection }).await
}

/// The calling window could not apply `skin` (`useSkin.ts`: an image did
/// not decode, or the stylesheet did not load) and kept the skin it
/// shows. The window only reports; the Host logs it, tells this window
/// and Settings, reverts `skin` if it is on trial (plan SK2-T1), and
/// answers with its words for the failure, which the window shows.
#[tauri::command]
pub async fn report_skin_apply_failed(
    state: State<'_, SharedState>,
    window: tauri::WebviewWindow,
    skin: String,
    cause: SkinApplyFailure,
) -> Result<Done<String>, CommandError> {
    if matches!(
        cause,
        SkinApplyFailure::WindowTooSmall { .. } | SkinApplyFailure::LoadTimeout { .. }
    ) {
        return Err(CommandError::Failed {
            code: 400,
            message: "Native skin failures come from the desktop shell".to_owned(),
            remediation: None,
        });
    }
    let window = window.label().to_owned();
    host_call::call(
        &state,
        IpcRequest::SkinApplyFailed {
            window,
            skin: Some(skin),
            cause,
        },
    )
    .await
}

/// The terminal result of this document's native reveal gate.
#[tauri::command]
pub async fn wait_skin_open(window: WebviewWindow) -> Result<Option<&'static str>, &'static str> {
    crate::windows::skin_load::wait_open(&window).await
}

/// Claim the native skin apply before the webview asks its image and CSS loaders.
#[tauri::command]
pub async fn start_skin_apply(
    window: WebviewWindow,
    serial: u32,
    skin: String,
    href: String,
    expired: tauri::ipc::Channel<&'static str>,
) -> Option<String> {
    crate::windows::skin_load::start_apply(&window, serial, &skin, &href, expired)
}

/// A completed apply cancels its watchdog. A committed sheet also joins
/// the native allocation at the reveal gate.
#[tauri::command]
pub async fn finish_skin_apply(window: WebviewWindow, attempt_id: String, href: Option<String>) {
    crate::windows::skin_load::finish_apply(&window, &attempt_id, href);
}
