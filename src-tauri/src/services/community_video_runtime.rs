//! Phase 23.C — video-handler Tauri-runtime orchestration lifted from
//! `commands/community/video.rs`. Hosts the per-handler inners plus a
//! shared `decode_stream_id` helper used by the four envelope-based
//! commands (frame_ack, keyframe_request, frame_send, topology_change).

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::services::community::video;
use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendVideoFrameRequest {
    pub stream_id_hex: String,
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Codec wire string ("vp9" | "vp8" | "h264") the frontend encoder
    /// produced this chunk with. Unknown values are a hard error.
    pub codec: String,
    pub timestamp: u32,
    pub encoded_payload_b64: String,
}

fn decode_stream_id(hex_str: &str) -> Result<[u8; 16], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("invalid stream_id hex: {e}"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| "stream_id must be 16 bytes".to_string())
}

pub fn derive_video_stream_id_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    track_label: &str,
) -> Result<String, String> {
    let pseudonym_hex = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
            .ok_or_else(|| "not a member of this community".to_string())?
    };
    let stream_id = rekindle_video::derive_stream_id(channel_id, &pseudonym_hex, track_label);
    Ok(hex::encode(stream_id))
}

pub fn send_video_frame_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    request: &SendVideoFrameRequest,
) -> Result<u32, String> {
    use base64::Engine as _;
    let stream_id = decode_stream_id(&request.stream_id_hex)?;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(request.encoded_payload_b64.as_bytes())
        .map_err(|e| format!("invalid base64 payload: {e}"))?;
    if let Err(reason) = crate::services::community::media_ready_runtime::media_ready_gate(
        state,
        community_id,
        channel_id,
    ) {
        let n = crate::services::community::media_ready_runtime::note_pre_ready_drop(state);
        if n == 1 || n.is_multiple_of(30) {
            // One line per ~2s at 15fps — enough to diagnose, never spam.
            tracing::warn!(
                target: "rekindle_video::send",
                community_id,
                channel_id,
                dropped_total = n,
                %reason,
                "video frame dropped — media not ready"
            );
        }
        return Err(format!("media not ready: {reason}"));
    }
    let codec = rekindle_types::video::Codec::from_wire_str(&request.codec)
        .ok_or_else(|| format!("unknown codec wire string: {}", request.codec))?;
    let send_request = video::VideoFrameSend {
        stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        codec,
        timestamp: request.timestamp,
        encoded_payload: payload,
    };
    video::send_video_frame(state, community_id, channel_id, &send_request)
}

pub fn send_video_frame_ack_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    stream_id_hex: &str,
    last_frame_seq: u32,
    kbps: u32,
    loss_q8: u8,
) -> Result<(), String> {
    let stream_id = decode_stream_id(stream_id_hex)?;
    let envelope = CommunityEnvelope::Control(ControlPayload::FrameAck {
        channel_id: channel_id.to_string(),
        stream_id,
        last_frame_seq,
        kbps,
        loss_q8,
    });
    crate::services::community::send_to_channel_peers(state, community_id, channel_id, &envelope)
}

pub fn send_video_keyframe_request_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    stream_id_hex: &str,
) -> Result<(), String> {
    let stream_id = decode_stream_id(stream_id_hex)?;
    let envelope = CommunityEnvelope::Control(ControlPayload::KeyframeRequest {
        channel_id: channel_id.to_string(),
        stream_id,
    });
    crate::services::community::send_to_channel_peers(state, community_id, channel_id, &envelope)
}

pub fn send_video_bandwidth_estimate_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    kbps: u32,
    window_secs: u8,
    loss_q8: u8,
) -> Result<(), String> {
    let envelope = CommunityEnvelope::Control(ControlPayload::BandwidthEstimate {
        channel_id: channel_id.to_string(),
        kbps,
        window_secs,
        loss_q8,
    });
    crate::services::community::send_to_channel_peers(state, community_id, channel_id, &envelope)
}

pub fn notify_video_topology_change_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    stream_id_hex: &str,
    relay_host_pseudonym: Option<String>,
    reason: String,
) -> Result<(), String> {
    let stream_id = decode_stream_id(stream_id_hex)?;
    let lamport = state_helpers::increment_lamport(state, community_id);
    let envelope = CommunityEnvelope::Control(ControlPayload::TopologyChange {
        channel_id: channel_id.to_string(),
        stream_id,
        relay_host_pseudonym,
        reason,
        lamport,
    });
    crate::services::community::send_to_channel_peers(state, community_id, channel_id, &envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Phase 1 — the media-ready gate is the FIRST check: a default
    /// AppState (no voice session, no handshake) must reject the frame
    /// with the gate's reason and count the drop.
    #[test]
    fn send_video_frame_rejected_before_media_ready() {
        let state: SharedState = Arc::new(crate::state::AppState::default());
        let request = SendVideoFrameRequest {
            stream_id_hex: "00".repeat(16),
            frame_seq: 1,
            keyframe: true,
            codec: "vp9".to_string(),
            timestamp: 0,
            encoded_payload_b64: "AAAA".to_string(),
        };
        let err = send_video_frame_inner(&state, "c1", "ch1", &request)
            .expect_err("default state must be gated");
        assert!(
            err.starts_with("media not ready:"),
            "gate must name itself: got {err}"
        );
        assert!(
            err.contains("not-in-voice"),
            "reason names the blocker: {err}"
        );
        assert_eq!(
            state
                .video_pre_ready_drops
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "drop counter increments"
        );
    }
}
