//! Local FTS5 message search (architecture §23), answered by the Host
//! (plan E3.1 E3b).

use rekindle_ipc::protocol::PinRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::search::{MessageSearch, SearchResult};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn search_messages(
    request: MessageSearch,
    state: State<'_, SharedState>,
) -> Result<Done<SearchResult>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::MessageSearch {
            request,
        })),
    )
    .await
}
