//! Video / screen-share Tauri commands. Architecture §10.6 — the
//! actual VP9 capture + encode happens in the webview (WebCodecs);
//! the desktop side provides framing helpers and emits assembled
//! frames the renderer can consume.

use tauri::State;

use crate::services::community_video_runtime::{
    derive_video_stream_id_inner, notify_video_topology_change_inner,
    send_video_bandwidth_estimate_inner, send_video_frame_ack_inner, send_video_frame_inner,
    send_video_keyframe_request_inner,
};
use crate::state::SharedState;

pub use crate::services::community_video_runtime::SendVideoFrameRequest;

#[tauri::command]
pub async fn derive_video_stream_id(
    community_id: String,
    channel_id: String,
    track_label: String,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    derive_video_stream_id_inner(state.inner(), &community_id, &channel_id, &track_label)
}

/// Default §10.6 interim media capabilities (480p @ 15fps, VP9 only).
#[tauri::command]
pub async fn default_media_capabilities() -> Result<rekindle_video::MediaCapabilities, String> {
    Ok(rekindle_video::MediaCapabilities::interim_default())
}

#[tauri::command]
pub async fn send_video_frame(
    community_id: String,
    channel_id: String,
    request: SendVideoFrameRequest,
    state: State<'_, SharedState>,
) -> Result<u32, String> {
    send_video_frame_inner(state.inner(), &community_id, &channel_id, &request)
}

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches FrameAck envelope shape"
)]
pub async fn send_video_frame_ack(
    community_id: String,
    channel_id: String,
    stream_id_hex: String,
    last_frame_seq: u32,
    kbps: u32,
    loss_q8: u8,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    send_video_frame_ack_inner(
        state.inner(),
        &community_id,
        channel_id,
        &stream_id_hex,
        last_frame_seq,
        kbps,
        loss_q8,
    )
}

#[tauri::command]
pub async fn send_video_keyframe_request(
    community_id: String,
    channel_id: String,
    stream_id_hex: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    send_video_keyframe_request_inner(state.inner(), &community_id, channel_id, &stream_id_hex)
}

#[tauri::command]
pub async fn send_video_bandwidth_estimate(
    community_id: String,
    channel_id: String,
    kbps: u32,
    window_secs: u8,
    loss_q8: u8,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    send_video_bandwidth_estimate_inner(
        state.inner(),
        &community_id,
        channel_id,
        kbps,
        window_secs,
        loss_q8,
    )
}

#[tauri::command]
pub async fn notify_video_topology_change(
    community_id: String,
    channel_id: String,
    stream_id_hex: String,
    relay_host_pseudonym: Option<String>,
    reason: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    notify_video_topology_change_inner(
        state.inner(),
        &community_id,
        channel_id,
        &stream_id_hex,
        relay_host_pseudonym,
        reason,
    )
}
