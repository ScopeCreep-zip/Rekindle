use tauri::State;

use crate::db::DbPool;
use crate::services::status_runtime::{
    get_avatar_inner, set_avatar_inner, set_nickname_inner, set_status_inner,
    set_status_message_inner,
};
use crate::state::SharedState;

/// Set online status and publish to DHT.
#[tauri::command]
pub async fn set_status(status: String, state: State<'_, SharedState>) -> Result<(), String> {
    set_status_inner(state.inner(), status).await
}

/// Set display name, persist to `SQLite`, and push update to DHT.
#[tauri::command]
pub async fn set_nickname(
    nickname: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    set_nickname_inner(state.inner(), pool.inner(), &app, nickname).await
}

/// Set avatar image: compress to WebP, persist to `SQLite`, and push update to DHT.
#[tauri::command]
pub async fn set_avatar(
    avatar_data: Vec<u8>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    set_avatar_inner(state.inner(), pool.inner(), &app, avatar_data).await
}

/// Retrieve a user's avatar as WebP bytes.
#[tauri::command]
pub async fn get_avatar(
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Option<Vec<u8>>, String> {
    get_avatar_inner(state.inner(), pool.inner(), public_key).await
}

/// Set status message and push update to DHT.
#[tauri::command]
pub async fn set_status_message(
    message: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    set_status_message_inner(state.inner(), message).await
}
