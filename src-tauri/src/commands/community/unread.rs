use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;

use super::types::UnreadCountEntry;

#[tauri::command]
pub async fn mark_channel_read(
    community_id: String,
    channel_id: String,
    last_message_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = (pool, last_message_id);

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.unread_count = 0;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_unread_counts(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<UnreadCountEntry>, String> {
    let communities = state.communities.read();
    let community = communities
        .get(&community_id)
        .ok_or("community not found")?;
    Ok(community
        .channels
        .iter()
        .map(|ch| UnreadCountEntry {
            channel_id: ch.id.clone(),
            unread_count: ch.unread_count,
        })
        .collect())
}
