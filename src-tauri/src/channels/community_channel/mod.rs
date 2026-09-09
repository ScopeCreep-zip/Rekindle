use serde::Serialize;

mod dto;
#[cfg(target_os = "linux")]
pub use dto::NativeVideoErrorEvent;
pub use dto::{
    EventInfoDto, EventRsvpInfoDto, GameServerInfoDto, RoleDto, ThreadInfoDto,
    VideoBandwidthEstimateEvent, VideoBitrateTargetEvent, VideoCodecIncompatibleEvent,
    VideoEnvelopeRejectedEvent, VideoFrameAckEvent, VideoKeyframeRequestEvent,
    VideoMediaCapabilitiesEvent, VideoSessionConfigEvent, VideoTopologyChangeEvent,
};

/// One voice-roster participant as shipped to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRosterParticipantEvent {
    pub pseudonym_key: String,
    pub display_name: Option<String>,
}

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
    /// A member started typing in a channel.
    #[serde(rename_all = "camelCase")]
    ChannelTyping {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// Architecture §10.9: a member triggered a soundboard sound in a
    /// voice channel. The frontend looks up the expression in the local
    /// cache and plays the audio.
    #[serde(rename_all = "camelCase")]
    SoundboardPlay {
        community_id: String,
        channel_id: String,
        expression_id: String,
        actor_pseudonym: String,
    },
    /// Architecture §10.6 receiver acknowledgement (see
    /// [`VideoFrameAckEvent`]).
    VideoFrameAck(VideoFrameAckEvent),
    /// Architecture §10.6 — receiver requests an I-frame (see
    /// [`VideoKeyframeRequestEvent`]).
    VideoKeyframeRequest(VideoKeyframeRequestEvent),
    /// Architecture §10.6 — receiver bandwidth advertisement (see
    /// [`VideoBandwidthEstimateEvent`]).
    VideoBandwidthEstimate(VideoBandwidthEstimateEvent),
    /// Architecture §10.6 line 4084 — peer capability advertisement
    /// (see [`VideoMediaCapabilitiesEvent`]).
    VideoMediaCapabilities(VideoMediaCapabilitiesEvent),
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
    /// A member joined a voice channel.
    #[serde(rename_all = "camelCase")]
    VoiceJoin {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
        route_blob: Vec<u8>,
        /// Name carried by the join handshake — render without waiting
        /// for the registry scan.
        display_name: Option<String>,
    },
    /// A member left a voice channel.
    #[serde(rename_all = "camelCase")]
    VoiceLeave {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// Full voice-channel roster sent to a joiner so it sees everyone
    /// already present (decoupled from MEK-decrypt). §10.1/§10.5.
    #[serde(rename_all = "camelCase")]
    VoiceRoster {
        community_id: String,
        channel_id: String,
        participants: Vec<VoiceRosterParticipantEvent>,
    },
    /// Local three-way voice join handshake progressed
    /// ("seen" | "connected"). `peer`/`display_name` identify the
    /// member whose evidence drove the transition.
    #[serde(rename_all = "camelCase")]
    VoiceJoinHandshake {
        community_id: String,
        channel_id: String,
        state: String,
        peer: Option<String>,
        display_name: Option<String>,
    },
    /// A joiner completed its handshake (transport-ready) — the UI
    /// renders them solid instead of pending.
    #[serde(rename_all = "camelCase")]
    VoicePeerConfirmed {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// Media-ready gate state for the active voice/video session
    /// (WebRTC "transport before RTP" analog). Emitted on every
    /// (ready, reason) transition; `reason` names the next blocker
    /// ("handshake-announced" → … → "ready"). The frontend disables
    /// camera/screen-share until `ready` and the backend hard-rejects
    /// video egress.
    #[serde(rename_all = "camelCase")]
    VoiceMediaReady {
        community_id: String,
        channel_id: String,
        ready: bool,
        reason: String,
    },
    /// Voice channel mode switched (mesh ↔ MCU).
    #[serde(rename_all = "camelCase")]
    VoiceModeSwitch {
        community_id: String,
        channel_id: String,
        mode: String,
        host_pseudonym: Option<String>,
    },
    /// Stage channel speaker/topic update.
    #[serde(rename_all = "camelCase")]
    StageUpdate {
        community_id: String,
        channel_id: String,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator_pseudonym: String,
    },
    /// Local moderator-facing notification for a speak request.
    #[serde(rename_all = "camelCase")]
    SpeakRequest {
        community_id: String,
        channel_id: String,
        requester_pseudonym: String,
    },
    /// Response to our stage speak request.
    #[serde(rename_all = "camelCase")]
    SpeakResponse {
        community_id: String,
        channel_id: String,
        requester_pseudonym: String,
        granted: bool,
        moderator_pseudonym: String,
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
