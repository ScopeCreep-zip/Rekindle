//! The user's preferences (plan D1): per identity, in its database
//! (`app_settings`, `app_input_channels`), so they reset with it (D9).
//! Launch-at-login is not here: it is the OS login item's state, which the
//! desktop shell reads from its autostart plugin.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Every preference a host keeps for the identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub notifications_enabled: bool,
    pub notification_sound: bool,
    pub start_minimized: bool,
    pub game_detection_enabled: bool,
    /// Seconds between game scans, 1–3600.
    pub game_scan_interval_secs: u32,
    /// Selected input device name (`None` = system default).
    pub input_device: Option<String>,
    /// Selected output device name (`None` = system default).
    pub output_device: Option<String>,
    /// Selected camera deviceId (WebView MediaDevices; `None` = default).
    pub video_device_id: Option<String>,
    /// Selected camera label: WebKit deviceIds are origin/data-store salted
    /// and rotate across reinstalls, so the label is the stable key.
    /// Selection resolves id first, then label, then the default.
    pub video_device_label: Option<String>,
    /// Input volume multiplier, 0.0–1.0.
    pub input_volume: f32,
    /// Input-channel choice per input device name, 0-based (plan C7.24b,
    /// Mumble's input channel mask). A device without an entry averages
    /// all its channels.
    pub input_channels: BTreeMap<String, Vec<u16>>,
    /// Output volume multiplier, 0.0–1.0.
    pub output_volume: f32,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    /// Minutes of inactivity before auto-away, 0 (off) to 1440.
    pub auto_away_minutes: u32,
    /// Accepting a friend request also volunteers a Strand Relay route for
    /// that friend. Off by default: explicit consent, per friend, never
    /// network-wide (W11.3).
    pub auto_volunteer_relay_for_new_friends: bool,
    /// The synthesized ring and ringback, independent of message sounds.
    pub ringtone_enabled: bool,
    /// Ringtone volume, 0.0–1.0.
    pub ringtone_volume: f32,
    /// Silence OS notifications and message sounds during a call.
    pub in_call_dnd_auto_enable: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            notifications_enabled: true,
            notification_sound: true,
            start_minimized: false,
            game_detection_enabled: true,
            game_scan_interval_secs: 15,
            input_device: None,
            output_device: None,
            video_device_id: None,
            video_device_label: None,
            input_volume: 1.0,
            input_channels: BTreeMap::new(),
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
