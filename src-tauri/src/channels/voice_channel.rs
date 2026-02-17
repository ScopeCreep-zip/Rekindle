use serde::Serialize;

/// Events streamed from Rust to the frontend for voice channels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum VoiceEvent {
    #[serde(rename_all = "camelCase")]
    UserJoined {
        public_key: String,
        display_name: String,
    },
    #[serde(rename_all = "camelCase")]
    UserLeft {
        public_key: String,
    },
    #[serde(rename_all = "camelCase")]
    UserSpeaking {
        public_key: String,
        speaking: bool,
    },
    #[serde(rename_all = "camelCase")]
    UserMuted {
        public_key: String,
        muted: bool,
    },
    ConnectionQuality {
        quality: String,
    },
    #[serde(rename_all = "camelCase")]
    DeviceChanged {
        device_type: String,
        device_name: String,
        reason: String,
    },
}
