//! Video / screen-share Tauri commands. Architecture §10.6 — the
//! actual VP9 capture + encode happens in the webview (WebCodecs);
//! the desktop side provides framing helpers and emits assembled
//! frames the renderer can consume.

use tauri::State;

use crate::services::community::video;
use crate::state::SharedState;

/// Compute the deterministic 16-byte stream_id the frontend must use
/// when fragmenting outbound video. Architecture §10.6 line 2063
/// derives stream_id from `(channel_id || sender_pseudonym ||
/// track_label)` so concurrent streams in the same channel never
/// collide and a single member can stream camera + screen share
/// simultaneously. `track_label` is one of `"camera"` / `"screen"`
/// (or any UTF-8 string the caller picks).
#[tauri::command]
pub async fn derive_video_stream_id(
    community_id: String,
    channel_id: String,
    track_label: String,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let pseudonym_hex = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
            .ok_or_else(|| "not a member of this community".to_string())?
    };
    let stream_id = video::derive_stream_id(&channel_id, &pseudonym_hex, &track_label);
    Ok(hex::encode(stream_id))
}

/// Default §10.6 interim media capabilities (480p @ 15fps, VP9 only)
/// for clients that don't introspect their hardware.
#[tauri::command]
pub async fn default_media_capabilities() -> Result<rekindle_video::MediaCapabilities, String> {
    Ok(rekindle_video::MediaCapabilities::interim_default())
}

/// Send one encoded video frame to the community mesh
/// (architecture §10.6). The frontend produces VP9 chunks via
/// WebCodecs and base64-encodes them for IPC; the backend MEK-encrypts,
/// fragments to ≤28 KB, attaches FEC parity for keyframes, signs each
/// fragment, and broadcasts via gossip. Returns the number of fragments
/// dispatched (data + parity combined).
#[tauri::command]
pub async fn send_video_frame(
    community_id: String,
    channel_id: String,
    request: SendVideoFrameRequest,
    state: State<'_, SharedState>,
) -> Result<u32, String> {
    use base64::Engine as _;
    let stream_id_bytes =
        hex::decode(&request.stream_id_hex).map_err(|e| format!("invalid stream_id hex: {e}"))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "stream_id must be 16 bytes".to_string())?;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(request.encoded_payload_b64.as_bytes())
        .map_err(|e| format!("invalid base64 payload: {e}"))?;
    let send_request = video::VideoFrameSend {
        stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        timestamp: request.timestamp,
        encoded_payload: payload,
    };
    video::send_video_frame(state.inner(), &community_id, &channel_id, &send_request)
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendVideoFrameRequest {
    pub stream_id_hex: String,
    pub frame_seq: u32,
    pub keyframe: bool,
    pub timestamp: u32,
    pub encoded_payload_b64: String,
}

/// Architecture §10.6 line 4081 — receiver acks frames roughly every
/// 500 ms with measured downstream kbps + loss so senders can adapt
/// their VP9 bitrate. `loss_q8` is fixed-point 0..=255 (0 = perfect,
/// 255 = total loss).
#[tauri::command]
pub async fn send_video_frame_ack(
    community_id: String,
    channel_id: String,
    stream_id_hex: String,
    last_frame_seq: u32,
    kbps: u32,
    loss_q8: u8,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    let stream_id_bytes =
        hex::decode(&stream_id_hex).map_err(|e| format!("invalid stream_id hex: {e}"))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "stream_id must be 16 bytes".to_string())?;
    let envelope = CommunityEnvelope::Control(ControlPayload::FrameAck {
        channel_id,
        stream_id,
        last_frame_seq,
        kbps,
        loss_q8,
    });
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

/// Architecture §10.6 line 4081 — receiver lost too many inter-frames
/// and asks the sender to mark the next frame as a keyframe.
#[tauri::command]
pub async fn send_video_keyframe_request(
    community_id: String,
    channel_id: String,
    stream_id_hex: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    let stream_id_bytes =
        hex::decode(&stream_id_hex).map_err(|e| format!("invalid stream_id hex: {e}"))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "stream_id must be 16 bytes".to_string())?;
    let envelope = CommunityEnvelope::Control(ControlPayload::KeyframeRequest {
        channel_id,
        stream_id,
    });
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

/// Architecture §10.6 line 4082 — out-of-band bandwidth advertisement
/// when network conditions change between frames (Wi-Fi → cellular).
#[tauri::command]
pub async fn send_video_bandwidth_estimate(
    community_id: String,
    channel_id: String,
    kbps: u32,
    window_secs: u8,
    loss_q8: u8,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    let envelope = CommunityEnvelope::Control(ControlPayload::BandwidthEstimate {
        channel_id,
        kbps,
        window_secs,
        loss_q8,
    });
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

/// Architecture §10.6 Phase 6 Week 22 — broadcast that the active
/// video relay for a `(channel_id, stream_id)` has changed. The
/// outgoing or incoming relay calls this so peers re-attach their
/// decoders and reset reassembly buffers. `relay_host_pseudonym` is
/// `None` when reverting to direct mesh delivery.
#[tauri::command]
pub async fn notify_video_topology_change(
    community_id: String,
    channel_id: String,
    stream_id_hex: String,
    relay_host_pseudonym: Option<String>,
    reason: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

    let stream_id_bytes =
        hex::decode(&stream_id_hex).map_err(|e| format!("invalid stream_id hex: {e}"))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "stream_id must be 16 bytes".to_string())?;
    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    let envelope = CommunityEnvelope::Control(ControlPayload::TopologyChange {
        channel_id,
        stream_id,
        relay_host_pseudonym,
        reason,
        lamport,
    });
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}
