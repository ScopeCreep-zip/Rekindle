use serde::Serialize;

/// Events streamed from Rust to the frontend for voice channels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum VoiceEvent {
    /// Emitted on the local node when WE successfully join a voice channel.
    /// Carries `active_call_type` so any frontend (Tauri GUI, CLI, future TUI)
    /// can mirror the same authoritative state without per-frontend logic —
    /// fixes C1 (video panel never mounts because activeCallType wasn't set).
    #[serde(rename_all = "camelCase")]
    LocalJoined {
        channel_id: String,
        active_call_type: String,
    },
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
