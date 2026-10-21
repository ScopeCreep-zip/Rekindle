//! Group DMs (plan E2.6): each command is one Host request; the Host
//! validates it, reviews its intent and runs `operations::group_dm`.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::group_dm::GroupDmView;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Create a group of us and `members` (2-7 friends) named `name`
/// (`IpcRequest::GroupDmCreate`); the answer is its id.
#[tauri::command]
pub async fn group_dm_create(
    name: String,
    members: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<Done<String>, CommandError> {
    host_call::call(&state, IpcRequest::GroupDmCreate { name, members }).await
}

/// Accept a pending group invite (`IpcRequest::GroupDmAccept`).
#[tauri::command]
pub async fn group_dm_accept(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(&state, IpcRequest::GroupDmAccept { group_id }).await
}

/// Decline a pending group invite (`IpcRequest::GroupDmDecline`).
#[tauri::command]
pub async fn group_dm_decline(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(&state, IpcRequest::GroupDmDecline { group_id }).await
}

/// Leave a group (`IpcRequest::GroupDmLeave`).
#[tauri::command]
pub async fn group_dm_leave(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(&state, IpcRequest::GroupDmLeave { group_id }).await
}

/// Every group we are in and every unanswered invite
/// (`IpcRequest::GroupDmList`).
#[tauri::command]
pub async fn group_dm_list(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<GroupDmView>>, CommandError> {
    host_call::call(&state, IpcRequest::GroupDmList).await
}

/// The newest `limit` messages of a group before `before` (ms), oldest
/// first (`IpcRequest::GroupDmMessages`).
#[tauri::command]
pub async fn group_dm_messages(
    group_id: String,
    limit: u32,
    before: Option<i64>,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<DmMessage>>, CommandError> {
    let request = IpcRequest::GroupDmMessages {
        group_id,
        limit,
        before,
    };
    host_call::call(&state, request).await
}
