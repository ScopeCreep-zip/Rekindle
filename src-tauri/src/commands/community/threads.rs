use tauri::State;

use crate::commands::chat::Message;
use crate::db::DbPool;
use crate::state::SharedState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::require_permission;

pub use crate::channels::community_channel::ThreadInfoDto;

#[tauri::command]
pub async fn create_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    name: String,
    starter_message_id: String,
    forum_tag: Option<String>,
    auto_archive_seconds: Option<u64>,
) -> Result<String, String> {
    crate::services::community::threads::create_thread(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        &name,
        &starter_message_id,
        forum_tag,
        auto_archive_seconds,
    )
    .await
}

#[tauri::command]
pub async fn get_channel_threads(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<ThreadInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    crate::services::community::threads::list_threads(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
    )
    .await
}

#[tauri::command]
pub async fn get_active_threads(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<ThreadInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    crate::services::community::threads::list_active_threads(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
    )
    .await
}

#[tauri::command]
pub async fn send_thread_message(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
    body: String,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    crate::services::community::threads::send_thread_message(
        state.inner(),
        &community_id,
        &thread_id,
        &body,
    )
    .await
}

#[tauri::command]
pub async fn get_thread_messages(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
    limit: u32,
    before_timestamp: Option<u64>,
) -> Result<Vec<Message>, String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    crate::services::community::threads::load_thread_messages(
        state.inner(),
        &community_id,
        &thread_id,
        limit,
        before_timestamp,
    )
    .await
}

#[tauri::command]
pub async fn archive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::MANAGE_THREADS)?;
    crate::services::community::threads::archive_thread(state.inner(), &community_id, &thread_id)
        .await
}

#[tauri::command]
pub async fn unarchive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    let _ = (state, pool, community_id, thread_id);
    Err("archived threads reactivate when a new message is sent".into())
}
