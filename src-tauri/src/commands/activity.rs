//! The local activity feed (plan E7.15, Infoview design §2.3): one page of
//! it, and keeping it or not. Both are the Host's requests; the shell only
//! carries them.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::activity::ActivityListItem;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::preferences::Preferences;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// One page of the feed, newest first: up to `limit` entries after row
/// `before` (`IpcRequest::ActivityList`), names as the Host holds them now.
#[tauri::command]
pub async fn activity_list(
    before: Option<i64>,
    limit: u16,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<ActivityListItem>>, CommandError> {
    host_call::call(&state, IpcRequest::ActivityList { before, limit }).await
}

/// Keep the local feed or stop (`IpcRequest::SetKeepActivityLog`): off
/// clears it. The preferences come back.
#[tauri::command]
pub async fn set_keep_activity_log(
    enabled: bool,
    state: State<'_, SharedState>,
) -> Result<Done<Preferences>, CommandError> {
    host_call::call(&state, IpcRequest::SetKeepActivityLog { enabled }).await
}
