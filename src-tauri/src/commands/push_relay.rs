//! Tauri commands for the mobile push relay (architecture §17.3).
//!
//! Desktop builds expose these primarily for testing; the actual mobile
//! integration ships as a separate `rekindle-push-relay` binary plus the
//! mobile app. The desktop app is also a valid relay client (the wake
//! signal triggers a foreground sync via Veilid `app_message`).

use tauri::State;

use crate::db::DbPool;
use crate::services::push_relay;
use crate::state::SharedState;

#[tauri::command]
pub async fn register_with_push_relay(
    relay_pseudonym: String,
    device_push_token: String,
    platform: String,
    record_keys: Vec<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    push_relay::register_with_push_relay(
        state.inner(),
        pool.inner(),
        &relay_pseudonym,
        &device_push_token,
        &platform,
        &record_keys,
    )
    .await
}

#[tauri::command]
pub async fn unregister_with_push_relay(
    relay_pseudonym: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    push_relay::unregister_with_push_relay(state.inner(), pool.inner(), &relay_pseudonym).await
}

#[tauri::command]
pub async fn list_push_relay_registrations(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<(String, String, String)>, String> {
    Ok(push_relay::list_registrations(state.inner(), pool.inner()).await)
}
