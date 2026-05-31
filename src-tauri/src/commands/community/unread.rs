use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;

use crate::services::community_unread_runtime::UnreadCountEntry;

#[tauri::command]
pub async fn mark_channel_read(
    community_id: String,
    channel_id: String,
    last_message_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = (pool, last_message_id);
    crate::services::community_unread_runtime::mark_channel_read_inner(
        state.inner(),
        &community_id,
        &channel_id,
    );
    Ok(())
}

#[tauri::command]
pub async fn get_unread_counts(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<UnreadCountEntry>, String> {
    crate::services::community_unread_runtime::get_unread_counts_inner(state.inner(), &community_id)
}
