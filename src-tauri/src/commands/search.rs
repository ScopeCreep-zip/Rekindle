//! Local FTS5 message search (architecture §23), answered by the Host
//! (plan E3.1 E3b), and the view-scoped local search (plan E7.2 G11).

use rekindle_ipc::protocol::PinRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::local_search::{LocalSearchResult, LocalSearchScope};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::search::{MessageSearch, SearchResult};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    pub async fn search_messages(
        request: MessageSearch,
        state: State<'_, SharedState>,
    ) -> Result<Done<SearchResult>, CommandError> {
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::MessageSearch {
            request,
        }))
    }
}

host_command! {
    state;
    /// Search what we already hold, scoped by the window's view (plan E7.2
    /// G11, SK2-G §8): friends, groups and members, our servers, our held
    /// files, or one direct conversation's messages; never the network.
    pub async fn local_search(
        scope: LocalSearchScope,
        query: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<LocalSearchResult>, CommandError> {
        IpcRequest::LocalSearch { scope, query }
    }
}
