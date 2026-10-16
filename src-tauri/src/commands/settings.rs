use rekindle_types::preferences::Preferences;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_autostart::ManagerExt as _;

use crate::state::SharedState;

/// The preferences as the settings window edits them: the identity's
/// stored preferences plus launch-at-login, which is the OS login item's
/// state (read back from the autostart plugin, never stored).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesDto {
    #[serde(flatten)]
    pub prefs: Preferences,
    pub auto_start: bool,
}

/// The unlocked identity's preferences and the OS login item's state.
#[tauri::command]
pub async fn get_preferences(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<PreferencesDto, String> {
    state.db.current()?;
    Ok(PreferencesDto {
        prefs: crate::services::preferences::current(state.inner()),
        auto_start: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
    })
}

/// Save the preferences, applying `auto_start` to the OS login items
/// first so a failed registration is reported instead of being stored as
/// if it had taken effect.
#[tauri::command]
pub async fn set_preferences(
    prefs: PreferencesDto,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    apply_auto_start(&app, prefs.auto_start)?;
    crate::services::preferences::update(state.inner(), |stored| *stored = prefs.prefs).await
}

fn apply_auto_start(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    if launcher.is_enabled().map_err(|e| e.to_string())? == enabled {
        return Ok(());
    }
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|e| format!("could not update launch-at-login: {e}"))
}

/// Check for application updates.
#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<bool, String> {
    // TODO: Check for updates via tauri-plugin-updater
    let current_version = env!("CARGO_PKG_VERSION");
    tracing::info!(version = current_version, "checking for updates");

    // Notify frontend if an update is available
    // In production, this compares versions from the update server
    let has_update = false;
    if has_update {
        let event = rekindle_types::subscription_events::NotificationEvent::UpdateAvailable {
            version: "0.2.0".to_string(),
        };
        crate::event_dispatch::emit_notification(&app, event);
    }

    Ok(has_update)
}
