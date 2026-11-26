//! Community voice DTOs (plan E4.1): the Host's answers to `VoiceDevices`
//! and `VoiceStatus`, rendered by the frontend from E4.1 S5.

use serde::{Deserialize, Serialize};

use crate::subscription_events::VoiceScope;

/// One audio device in a settings list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDevice {
    /// Stable id (cpal `DeviceId`): what a choice saves.
    pub id: String,
    /// Display name.
    pub name: String,
    pub is_default: bool,
    /// Input channels an input-channel choice picks from (plan C7.24b);
    /// 0 for output devices.
    pub channels: u16,
}

/// The Host's audio devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDevices {
    pub input_devices: Vec<VoiceDevice>,
    pub output_devices: Vec<VoiceDevice>,
}

/// One audio end of the running session (plan E4.1 `AudioIo`, C7.24).
///
/// Externally tagged, like `VoiceEvent`: it crosses the daemon IPC as
/// postcard, which is not self-describing, and an adjacently tagged enum
/// does not decode there (plan E4.1 S5a1 round-trip test).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum VoiceEndState {
    /// A device is open. `standsInFor` is the saved choice it replaces
    /// while that device is not connected (announced to the user).
    Device {
        id: String,
        name: String,
        stands_in_for: Option<String>,
    },
    /// No device could be opened: the visible C7.24 state the user
    /// resolves (listen-only without a mic, silent without a speaker).
    Unavailable { reason: String },
    /// Configured without a device (`Silence`/`Null`: a headless Host).
    Headless,
}

/// The Host's voice state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    /// The one live voice session (1:1 call, group call or community
    /// voice, lead ruling 2026-11-26); `None` when idle, and the rest then
    /// reads idle.
    pub session: Option<VoiceScope>,
    /// The Host's words for why a new voice session cannot start now (the
    /// precedence reason), or `None`. Every voice-start control reads it;
    /// none computes it.
    pub start_refusal: Option<String>,
    pub muted: bool,
    pub deafened: bool,
    pub input: VoiceEndState,
    pub output: VoiceEndState,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape the frontend renders.
    #[test]
    fn voice_status_serializes_in_the_ipc_shape() {
        let status = VoiceStatus {
            session: Some(VoiceScope::Community {
                community: "c".into(),
                channel: "v".into(),
            }),
            start_refusal: Some("Leave voice in c first.".into()),
            muted: true,
            deafened: false,
            input: VoiceEndState::Device {
                id: "builtin".into(),
                name: "Built-in".into(),
                stands_in_for: Some("usb".into()),
            },
            output: VoiceEndState::Unavailable {
                reason: "no output device available".into(),
            },
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "session": { "community": { "community": "c", "channel": "v" } },
                "startRefusal": "Leave voice in c first.",
                "muted": true,
                "deafened": false,
                "input": { "device": {
                    "id": "builtin", "name": "Built-in", "standsInFor": "usb" } },
                "output": { "unavailable": {
                    "reason": "no output device available" } }
            })
        );
        assert_eq!(
            serde_json::to_value(VoiceEndState::Headless).unwrap(),
            serde_json::json!("headless")
        );
        let back: VoiceStatus = serde_json::from_value(json).unwrap();
        assert_eq!(back, status);
    }
}
