use serde::{Deserialize, Serialize};
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
    /// Selected camera deviceId (WebView MediaDevices). None = system default.
    #[serde(default)]
    pub video_device_id: Option<String>,
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
    /// W11.3 — when ON, accepting a friend request also volunteers a
    /// Strand Relay route for that friend so they can route via you
    /// when their direct route is unavailable. OFF by default
    /// (explicit consent per `feedback_vulnerable_users_no_creative_paths.md`).
    /// Per-friend, never network-wide — chiral §28.4 invite-gated
    /// model. The toggle does not retroactively volunteer for
    /// existing friends; users opt those in via the friend context
    /// menu.
    #[serde(default)]
    pub auto_volunteer_relay_for_new_friends: bool,
    /// Wave 12 W12.2 — gates the synthesized incoming-call ring and
    /// outgoing ringback. Independent of `notification_sound` (which
    /// covers message dings) so the user can silence one without the
    /// other. Default ON.
    #[serde(default = "default_true")]
    pub ringtone_enabled: bool,
    /// Wave 12 W12.2 — linear volume for ringtone / ringback / busy
    /// tone, [0, 1]. Default 0.4 (matches the synth lib's clamp).
    #[serde(default = "default_ringtone_volume")]
    pub ringtone_volume: f32,
    /// Wave 12 W12.2 — when ON, suppresses OS notifications and
    /// message-arrival sounds while a call is active so a noisy chat
    /// doesn't distract participants. In-app modals still surface.
    #[serde(default = "default_true")]
    pub in_call_dnd_auto_enable: bool,
}

fn default_ringtone_volume() -> f32 {
    0.4
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
            video_device_id: None,
            input_volume: 1.0,
            output_volume: 1.0,
            noise_suppression: true,
            echo_cancellation: true,
            auto_away_minutes: 10,
            auto_volunteer_relay_for_new_friends: false,
            ringtone_enabled: true,
            ringtone_volume: 0.4,
            in_call_dnd_auto_enable: true,
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
        crate::event_dispatch::emit_live(&app, "notification-event", &event);
    }

    Ok(has_update)
}
