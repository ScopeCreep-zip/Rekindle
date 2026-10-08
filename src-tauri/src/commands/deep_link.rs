//! Consent for OS deep links. See `crate::deep_links`.

use tauri::State;

use crate::deep_links::{self, DeepLinkOutcome, DeepLinkRequest};
use crate::keystore::KeystoreHandle;
use crate::state::SharedState;

/// The deep link awaiting consent, if any. The buddy list calls this on
/// mount and when `deep-link-action` fires.
#[tauri::command]
pub async fn get_pending_deep_link(
    state: State<'_, SharedState>,
) -> Result<Option<DeepLinkRequest>, String> {
    Ok(deep_links::pending_request(state.inner()))
}

/// Act on the pending deep link the user confirmed.
#[tauri::command]
pub async fn confirm_deep_link(
    request_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<DeepLinkOutcome, String> {
    let pool = state.db.current()?;
    deep_links::confirm(
        app,
        state.inner(),
        &pool,
        keystore_handle.inner(),
        &request_id,
    )
    .await
}

/// Drop the pending deep link the user declined.
#[tauri::command]
pub async fn dismiss_deep_link(
    request_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    deep_links::dismiss(state.inner(), &request_id)
}
