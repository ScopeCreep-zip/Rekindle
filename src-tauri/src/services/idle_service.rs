//! Idle/auto-away orchestration — AppState, presence publish, event
//! emit. Platform idle-time detection itself lives in `rekindle_idle`
//! (harvested out of this file; see
//! `docs/research/2026-10-harvest-security-infra-audit.md` — it had
//! zero `AppState`/Tauri dependency, same shape as
//! `rekindle-game-detect`'s `platform/` module).

use std::sync::Arc;

use tauri_plugin_store::StoreExt;
use tokio_util::sync::CancellationToken;

use crate::state::{AppState, UserStatus};
use crate::state_helpers;

/// Emit a presence status change event to the frontend.
fn emit_status_change(app_handle: &tauri::AppHandle, state: &Arc<AppState>, status: UserStatus) {
    let pk = state_helpers::owner_key_or_default(state);
    let status_str = match status {
        UserStatus::Online => "online",
        UserStatus::Away => "away",
        UserStatus::Busy => "busy",
        UserStatus::Offline | UserStatus::Invisible => "offline",
    };
    // Our own status, on the daemon vocabulary: the idle timer observed
    // the status and nothing about the game, so `game` stays unobserved
    // and the auto-away does not clear a running game.
    crate::event_dispatch::emit_subscription(
        app_handle,
        &rekindle_types::subscription_events::SubscriptionEvent::Presence(
            rekindle_types::subscription_events::PresenceEvent::SelfChanged {
                public_key: pk,
                snapshot: rekindle_types::subscription_events::PresenceSnapshot::status(status_str),
            },
        ),
    );
}

/// Run the idle/auto-away service until `stop` is cancelled.
///
/// Polls OS idle time every 30 seconds. When idle time exceeds the configured
/// `auto_away_minutes`, sets status to Away and stores the previous status.
/// When activity resumes, restores the previous status.
pub async fn run_idle_service(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    stop: CancellationToken,
) {
    tracing::info!("idle service started");

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            () = stop.cancelled() => break,
        }

        let auto_away_minutes = read_auto_away_minutes(&app_handle);
        if auto_away_minutes == 0 {
            continue;
        }
        let threshold = u64::from(auto_away_minutes) * 60;

        let Some(idle_secs) = tokio::task::spawn_blocking(rekindle_idle::get_idle_seconds)
            .await
            .ok()
            .flatten()
        else {
            tracing::warn!("idle service: get_idle_seconds returned None");
            continue;
        };

        let current_status = state_helpers::identity_status(&state);
        let is_auto_away = state.pre_away_status.read().is_some();

        tracing::debug!(
            idle_secs,
            threshold,
            ?current_status,
            is_auto_away,
            "idle service tick"
        );

        if idle_secs >= threshold && current_status == Some(UserStatus::Online) && !is_auto_away {
            // Activate auto-away
            *state.pre_away_status.write() = Some(UserStatus::Online);
            if let Some(ref mut id) = *state.identity.write() {
                id.status = UserStatus::Away;
            }
            crate::services::presence_service::request_status_publish(&state);
            emit_status_change(&app_handle, &state, UserStatus::Away);
            tracing::info!(idle_secs, "auto-away activated");
        } else if idle_secs < threshold && is_auto_away {
            // Restore previous status
            let restore = state
                .pre_away_status
                .write()
                .take()
                .unwrap_or(UserStatus::Online);
            if let Some(ref mut id) = *state.identity.write() {
                id.status = restore;
            }
            crate::services::presence_service::request_status_publish(&state);
            emit_status_change(&app_handle, &state, restore);
            tracing::info!(?restore, "auto-away deactivated");
        }
    }
    tracing::debug!("idle service shut down");
}

fn read_auto_away_minutes(app_handle: &tauri::AppHandle) -> u32 {
    let Ok(store) = app_handle.store("preferences.json") else {
        return 10;
    };
    store
        .get("preferences")
        .and_then(|v| v.get("autoAwayMinutes")?.as_u64())
        .map_or(10, |v| u32::try_from(v).unwrap_or(10))
}
