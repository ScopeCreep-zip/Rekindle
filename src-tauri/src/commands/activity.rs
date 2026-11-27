//! The local activity feed (plan E7.15, Infoview design §2.3): one page of
//! it, and keeping it or not. Both are the Host's requests; the shell only
//! carries them.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::activity::ActivityListItem;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::preferences::Preferences;
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    /// One page of the feed, newest first: up to `limit` entries after row
    /// `before` (`IpcRequest::ActivityList`), names as the Host holds them now.
    pub async fn activity_list(
        before: Option<i64>,
        limit: u16,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<ActivityListItem>>, CommandError> {
        IpcRequest::ActivityList { before, limit }
    }
}

host_command! {
    state;
    /// Keep the local feed or stop (`IpcRequest::SetKeepActivityLog`): off
    /// clears it. The preferences come back.
    pub async fn set_keep_activity_log(
        enabled: bool,
        state: State<'_, SharedState>,
    ) -> Result<Done<Preferences>, CommandError> {
        IpcRequest::SetKeepActivityLog { enabled }
    }
}
