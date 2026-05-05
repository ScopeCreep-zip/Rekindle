//! Tauri commands for the Strand Relay Network (architecture §13).
//!
//! Three operations exposed to the frontend: volunteer to relay for a
//! friend, revoke that volunteer offer, and list the offers other friends
//! have given us (so the UI can show "Carol is relaying for you").

use tauri::State;

use crate::db::DbPool;
use crate::services::relay;
use crate::state::SharedState;

#[tauri::command]
pub async fn volunteer_relay(
    friend_public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    relay::volunteer_relay(state.inner(), pool.inner(), &friend_public_key).await
}

#[tauri::command]
pub async fn revoke_relay(
    friend_public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    relay::revoke_relay(state.inner(), pool.inner(), &friend_public_key).await
}

#[tauri::command]
pub async fn list_received_relay_offers(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<String>, String> {
    Ok(relay::list_received_offers(state.inner(), pool.inner())
        .await
        .into_iter()
        .map(|(pseudonym, _blob)| pseudonym)
        .collect())
}

#[tauri::command]
pub async fn list_volunteered_relay_friends(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<String>, String> {
    Ok(relay::list_volunteered_for(state.inner(), pool.inner()).await)
}
