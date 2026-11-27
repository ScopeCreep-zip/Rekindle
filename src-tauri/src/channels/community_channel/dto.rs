//! Data-transfer types carried inside [`super::CommunityEvent`] variants.

use serde::Serialize;

use rekindle_video::SessionVideoConfig;

/// Architecture §10.6 — receiver requests an I-frame.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoKeyframeRequestEvent {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub channel_id: String,
    pub stream_id: String,
}

/// Architecture §10.6 — backend-negotiated per-call video config.
/// Recomputed and re-emitted whenever room membership changes or a
/// peer reports new capabilities. Frontend reconciles encoder +
/// decoder by tearing down and reconfiguring with the new
/// constraints. Backend owns the policy — CLI/TUI frontends inherit
/// the same `SessionVideoConfig` payload without re-running any
/// negotiation themselves.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSessionConfigEvent {
    pub community_id: String,
    pub channel_id: String,
    pub config: SessionVideoConfig,
}

/// Phase 3 — the negotiator found NO encoder codec every peer can
/// decode (`negotiate_session_config` returned `None` with a
/// non-empty local encode set). Emitted once per
/// compatible→incompatible transition (latched — membership churn
/// while incompatible does not re-emit). `peers` lists the
/// pseudonyms whose decode sets blocked every local encode codec.
/// Voice is unaffected; the frontend surfaces a toast.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCodecIncompatibleEvent {
    pub community_id: String,
    pub channel_id: String,
    pub peers: Vec<VideoCodecIncompatiblePeer>,
}

/// A blocking peer's community pseudonym and member-list name.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCodecIncompatiblePeer {
    pub pseudonym_key: String,
    pub display_name: String,
}

/// The allocator's video encoder target (plan E4.3.3): each media route's
/// estimate, audio first, video the rest, lowest over the routes with
/// video running. The webview encoder follows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoBitrateTargetEvent {
    pub community_id: String,
    pub channel_id: String,
    pub kbps: u32,
}

/// The native capture session died asynchronously (camera unplugged,
/// pipeline failure) — the panel reverts the camera toggle and surfaces
/// the message. Emitted by the native `rekindle-video-capture` pump on
/// every platform it runs (Linux GStreamer; macOS/Windows nokhwa+libvpx);
/// the webview encode path reports its own failures inline via `setError`
/// instead. The frontend TS union keeps the variant unconditionally.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeVideoErrorEvent {
    pub community_id: String,
    pub channel_id: String,
    pub message: String,
}

/// Phase F — a gossiped video envelope failed signature or shape
/// verification at the receive boundary. Surfaced to the UI so the
/// asymmetric-drop case (one peer rejects, the other doesn't) is
/// observable from frontend signals instead of grep.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoEnvelopeRejectedEvent {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub display_name: String,
    pub reason: String,
}

/// Architecture §10.6 + Phase 6 Week 22 — the active video relay
/// for a `(channel_id, stream_id)` changed. Frontend should
/// re-attach its decoder to the new relay's stream and discard any
/// partially-buffered frames from the old one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTopologyChangeEvent {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub channel_id: String,
    pub stream_id: String,
    pub relay_host_pseudonym: Option<String>,
    pub reason: String,
    pub lamport: u64,
}
