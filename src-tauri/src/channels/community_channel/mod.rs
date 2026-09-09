use serde::Serialize;

mod dto;
#[cfg(target_os = "linux")]
pub use dto::NativeVideoErrorEvent;
pub use dto::{
    EventInfoDto, EventRsvpInfoDto, GameServerInfoDto, RoleDto, ThreadInfoDto,
    VideoBitrateTargetEvent, VideoCodecIncompatibleEvent, VideoEnvelopeRejectedEvent,
    VideoKeyframeRequestEvent, VideoSessionConfigEvent, VideoTopologyChangeEvent,
};

/// Events streamed from Rust to the frontend for community operations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum CommunityEvent {
    /// Architecture §18.4 — eager-fetched expression bytes have landed in
    /// the local cache. Frontend should re-query `list_expressions` so
    /// the picker swaps the `:emojiname:` placeholder for the resolved
    /// inline_data_base64.
    #[serde(rename_all = "camelCase")]
    ExpressionAssetReady {
        community_id: String,
        expression_id: String,
    },
    /// A queued channel message was eventually delivered after retry.
    #[serde(rename_all = "camelCase")]
    ChannelMessageDelivered {
        community_id: String,
        channel_id: String,
        message_id: String,
    },
    /// A queued channel message permanently failed after all retry attempts.
    #[serde(rename_all = "camelCase")]
    ChannelMessageDeliveryFailed {
        community_id: String,
        channel_id: String,
        message_id: String,
    },
    /// Architecture §10.6 — receiver requests an I-frame (see
    /// [`VideoKeyframeRequestEvent`]).
    ///
    /// The only receiver-feedback message that reaches a frontend, and
    /// only because the webview may own the encoder: `FrameAck`,
    /// `BandwidthEstimate` and `MediaCapabilities` used to sit beside
    /// it here and were read by nothing — the backend consumes them in
    /// `apply_bitrate_feedback` / `on_peer_caps_received`. When the
    /// encoder moves behind the daemon this one follows them.
    /// See `docs/plans/video-media-engine.md`.
    VideoKeyframeRequest(VideoKeyframeRequestEvent),
    /// Architecture §10.6 — backend-negotiated per-call video config
    /// (see [`VideoSessionConfigEvent`]).
    VideoSessionConfig(VideoSessionConfigEvent),
    /// Phase 3 — no mutually-decodable encoder codec (see
    /// [`VideoCodecIncompatibleEvent`]).
    VideoCodecIncompatible(VideoCodecIncompatibleEvent),
    /// Phase 4 — backend bitrate policy target (see
    /// [`VideoBitrateTargetEvent`]).
    VideoBitrateTarget(VideoBitrateTargetEvent),
    /// Linux-native capture session died (see [`NativeVideoErrorEvent`]).
    #[cfg(target_os = "linux")]
    NativeVideoError(NativeVideoErrorEvent),
    /// Phase F — video envelope rejected at the receive boundary (see
    /// [`VideoEnvelopeRejectedEvent`]).
    VideoEnvelopeRejected(VideoEnvelopeRejectedEvent),
    /// Architecture §10.6 + Phase 6 W22 — active relay changed (see
    /// [`VideoTopologyChangeEvent`]).
    VideoTopologyChange(VideoTopologyChangeEvent),
    /// Architecture §28.8 — sender pre-fetched OpenGraph metadata for
    /// a URL embedded in `message_id`.
    #[serde(rename_all = "camelCase")]
    LinkPreviewReceived {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        message_id: String,
        url: String,
        title: Option<String>,
        description: Option<String>,
        image_url: Option<String>,
        site_name: Option<String>,
        fetched_at: u64,
    },
    /// Lost Cargo: a download finished — `local_path` is the on-disk file.
    /// Frontend updates the message bubble's "Download" button to "Open".
    #[serde(rename_all = "camelCase")]
    AttachmentDownloaded {
        community_id: String,
        channel_id: String,
        attachment_id: String,
        local_path: String,
    },
}

#[cfg(test)]
mod wire_tests;

pub mod onboarding_dto;
