//! The skins (plan E7.7.2): what a window applies (the windows re-apply
//! on `SystemEvent::SkinChanged`) and Settings → Appearance's library,
//! choice, trial and login skin. Host calls, one request each.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::skin::RegionFit;
use rekindle_types::dto::skin_draft::{SkinDraft, SkinDraftRegion, SkinDraftReport};
use rekindle_types::dto::skin_library::{SkinApplied, SkinEntry, SkinLibrary};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// What this window applies: `id`, or with `None` the skin this device
/// shows now (`IpcRequest::SkinGet`).
#[tauri::command]
pub async fn get_skin(
    id: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<SkinApplied>, CommandError> {
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
