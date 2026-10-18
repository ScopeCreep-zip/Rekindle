//! The window's side of the intent gate (plan E1.1, U1.6; ADR 0019): the
//! commit and cancel of an action the Host held for review, and the undo of
//! one that ran. Each is a Host call; the window only renders the review
//! and collects its proof.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::intent::{IntentId, IntentProof};
use tauri::State;
use tauri_plugin_opener::OpenerExt as _;

use crate::host_call;
use crate::state::SharedState;

/// Commit a held action with the proof its review asks for
/// (`IpcRequest::IntentCommit`). The answer is the action's own; a review
/// whose facts changed comes back as a fresh `IntentRequired`. A committed
/// `LinkOpen` this window asked for opens its URL here, the one opener.
#[tauri::command]
pub async fn intent_commit(
    intent_id: IntentId,
    proof: IntentProof,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let done: Done<serde_json::Value> =
        host_call::call(&state, IpcRequest::IntentCommit { intent_id, proof }).await?;
    if state.pending_link_opens.lock().remove(&intent_id) {
        if let Some(url) = done.value.as_str() {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| CommandError::internal(e.to_string()))?;
        }
    }
    Ok(done)
}

/// Drop a held action (`IpcRequest::IntentCancel`).
#[tauri::command]
pub async fn intent_cancel(
    intent_id: IntentId,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    state.pending_link_opens.lock().remove(&intent_id);
    host_call::call(&state, IpcRequest::IntentCancel { intent_id }).await
}

/// Take back an action that ran, while its offer lasts (`IpcRequest::Undo`).
#[tauri::command]
pub async fn undo(
    id: IntentId,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(&state, IpcRequest::Undo { id }).await
}
