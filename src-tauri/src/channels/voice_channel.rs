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
    UserLeft { public_key: String },
    #[serde(rename_all = "camelCase")]
    UserSpeaking { public_key: String, speaking: bool },
    #[serde(rename_all = "camelCase")]
    UserMuted { public_key: String, muted: bool },
    /// Merged 5 s health report: send-side quality classification +
    /// receive-side jitter drops + cumulative inbound-channel drops.
    #[serde(rename_all = "camelCase")]
    ConnectionQuality {
        quality: String,
        rx_overflow_drops: u64,
        rx_late_drops: u64,
        /// Inbound media dropped for MEK reasons (missing key /
        /// generation mismatch / decrypt failure) — the visible signal
        /// for a rotation race; never silent.
        rx_mek_drops: u64,
        ingress_drops: u64,
    },
    #[serde(rename_all = "camelCase")]
    DeviceChanged {
        device_type: String,
        device_name: String,
        reason: String,
    },
    /// Wave 14 W14.4 — voice packets dropped since last emit.
    /// Surfaces silent failures (channel full, no active call,
    /// AEAD decrypt fail) at info!/warn! log + as a frontend event.
    /// Backend-driven policy: emit every 1 s if count > 0.
    PacketsDropped { reason: String, count: u64 },
}
