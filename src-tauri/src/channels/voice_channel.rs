use serde::Serialize;

/// Events streamed from Rust to the frontend for voice channels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum VoiceEvent {
    UserJoined {
        public_key: String,
        display_name: String,
    },
    UserLeft {
        public_key: String,
    },
    UserSpeaking {
        public_key: String,
        speaking: bool,
    },
    UserMuted {
        public_key: String,
        muted: bool,
    },
    ConnectionQuality {
        quality: String,
    },
    DeviceChanged {
        device_type: String,
        device_name: String,
        reason: String,
    },
}
