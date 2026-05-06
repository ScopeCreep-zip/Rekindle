//! Tauri commands for local FTS5 message search (architecture §23).

use tauri::State;

use crate::db::DbPool;
use crate::services::search;
use crate::state::SharedState;
use rekindle_types::search::{MessageSearch, SearchResult};

#[tauri::command]
pub async fn search_messages(
    request: MessageSearch,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<SearchResult, String> {
    search::search_messages(state.inner(), pool.inner(), request).await
}
