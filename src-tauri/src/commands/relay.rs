//! Tauri commands for the Strand Relay Network (architecture §13).
//!
//! Three operations exposed to the frontend: volunteer to relay for a
//! friend, revoke that volunteer offer, and list the offers other friends
//! have given us (so the UI can show "Carol is relaying for you").

use tauri::State;

use crate::services::relay;
use crate::state::SharedState;

#[tauri::command]
pub async fn volunteer_relay(
    friend_public_key: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    relay::volunteer_relay(state.inner(), &pool, &friend_public_key).await
}

#[tauri::command]
pub async fn revoke_relay(
    friend_public_key: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    relay::revoke_relay(state.inner(), &pool, &friend_public_key).await
}

#[tauri::command]
pub async fn list_received_relay_offers(
    state: State<'_, SharedState>,
) -> Result<Vec<String>, String> {
    let pool = state.db.current()?;
    Ok(relay::list_received_offers(state.inner(), &pool)
        .await
        .into_iter()
        .map(|(pseudonym, _blob)| pseudonym)
        .collect())
}

#[tauri::command]
pub async fn list_volunteered_relay_friends(
    state: State<'_, SharedState>,
) -> Result<Vec<String>, String> {
    let pool = state.db.current()?;
    Ok(relay::list_volunteered_for(state.inner(), &pool).await)
}
