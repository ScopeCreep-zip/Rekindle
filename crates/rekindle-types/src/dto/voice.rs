//! Community voice DTOs (plan E4.1): the Host's answers to `VoiceDevices`
//! and `VoiceStatus`, rendered by the frontend from E4.1 S5.

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum VoiceEndState {
    /// A device is open. `standsInFor` is the saved choice it replaces
    /// while that device is not connected (announced to the user).
    #[serde(rename_all = "camelCase")]
    Device {
        id: String,
        name: String,
        stands_in_for: Option<String>,
    },
    /// No device could be opened: the visible C7.24 state the user
    /// resolves (listen-only without a mic, silent without a speaker).
    #[serde(rename_all = "camelCase")]
    Unavailable { reason: String },
    /// Configured without a device (`Silence`/`Null`: a headless Host).
    Headless,
}

/// The voice session the Host runs, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSessionRef {
    pub community_id: String,
    pub channel_id: String,
}

/// The Host's voice state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    /// `None` when not in a voice channel; the rest then reads idle.
    pub session: Option<VoiceSessionRef>,
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
            session: Some(VoiceSessionRef {
                community_id: "c".into(),
                channel_id: "v".into(),
            }),
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
                "session": { "communityId": "c", "channelId": "v" },
                "muted": true,
                "deafened": false,
                "input": { "type": "device", "data": {
                    "id": "builtin", "name": "Built-in", "standsInFor": "usb" } },
                "output": { "type": "unavailable", "data": {
                    "reason": "no output device available" } }
            })
        );
        assert_eq!(
            serde_json::to_value(VoiceEndState::Headless).unwrap(),
            serde_json::json!({ "type": "headless" })
        );
        let back: VoiceStatus = serde_json::from_value(json).unwrap();
        assert_eq!(back, status);
    }
}
