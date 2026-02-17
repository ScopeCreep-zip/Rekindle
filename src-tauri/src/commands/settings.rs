use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tauri_plugin_store::StoreExt;

use crate::channels::NotificationEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub notifications_enabled: bool,
    pub notification_sound: bool,
    pub start_minimized: bool,
    pub auto_start: bool,
    pub game_detection_enabled: bool,
    pub game_scan_interval_secs: u32,
    /// Selected input device name (None = system default).
    #[serde(default)]
    pub input_device: Option<String>,
    /// Selected output device name (None = system default).
    #[serde(default)]
    pub output_device: Option<String>,
    /// Input volume multiplier (0.0–1.0).
    #[serde(default = "default_volume")]
    pub input_volume: f32,
    /// Output volume multiplier (0.0–1.0).
    #[serde(default = "default_volume")]
    pub output_volume: f32,
    /// Whether noise suppression is enabled.
    #[serde(default = "default_true")]
    pub noise_suppression: bool,
    /// Whether echo cancellation is enabled.
    #[serde(default = "default_true")]
    pub echo_cancellation: bool,
    /// Minutes of inactivity before auto-away (0 = disabled).
    #[serde(default = "default_auto_away")]
    pub auto_away_minutes: u32,
}

fn default_volume() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_auto_away() -> u32 {
    10
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            notifications_enabled: true,
            notification_sound: true,
            start_minimized: false,
            auto_start: false,
            game_detection_enabled: true,
            game_scan_interval_secs: 15,
            input_device: None,
            output_device: None,
            input_volume: 1.0,
            output_volume: 1.0,
            noise_suppression: true,
            echo_cancellation: true,
            auto_away_minutes: 10,
        }
    }
}

#[tauri::command]
pub async fn get_preferences(app: tauri::AppHandle) -> Result<Preferences, String> {
    let store = app.store("preferences.json").map_err(|e| e.to_string())?;
    match store.get("preferences") {
        Some(val) => serde_json::from_value(val).map_err(|e| e.to_string()),
        None => Ok(Preferences::default()),
    }
}

#[tauri::command]
pub async fn set_preferences(prefs: Preferences, app: tauri::AppHandle) -> Result<(), String> {
    let store = app.store("preferences.json").map_err(|e| e.to_string())?;
    let val = serde_json::to_value(&prefs).map_err(|e| e.to_string())?;
    store.set("preferences", val);
    store.save().map_err(|e| e.to_string())?;
    Ok(())
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
        let event = NotificationEvent::UpdateAvailable {
            version: "0.2.0".to_string(),
        };
        let _ = app.emit("notification-event", &event);
    }

    Ok(has_update)
}
