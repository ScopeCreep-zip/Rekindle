//! The skins (plan E7.7.2): what a window applies (the windows re-apply
//! on `SystemEvent::SkinChanged`) and Settings → Appearance's library,
//! choice, trial and login skin. Host calls, one request each.

use rekindle_ipc::protocol::IpcRequest;
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
