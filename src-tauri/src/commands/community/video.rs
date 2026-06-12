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
        &channel_id,
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
    send_video_keyframe_request_inner(state.inner(), &community_id, &channel_id, &stream_id_hex)
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
        &channel_id,
        kbps,
        window_secs,
        loss_q8,
    )
}

/// Phase 11 Tier 1 — register the per-community `ipc::Channel` the video
/// panel listens on. Inbound reassembled frames for `community_id` are
/// pushed straight to this channel instead of the `community-event`
/// stream. Re-registering replaces the previous handle.
#[tauri::command]
pub async fn register_community_video_channel(
    community_id: String,
    on_frame: tauri::ipc::Channel<crate::video_channels::CommunityVideoFrameMsg>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    state
        .video_channels
        .register_community(community_id, on_frame);
    Ok(())
}

/// Phase 11 Tier 1 — drop the per-community video channel when the panel
/// unmounts so frames stop being forwarded.
#[tauri::command]
pub async fn unregister_community_video_channel(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    state.video_channels.unregister_community(&community_id);
    Ok(())
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
        &channel_id,
        &stream_id_hex,
        relay_host_pseudonym,
        reason,
    )
}

/// Architecture §10.6 Phase B — frontend reports the WebView's
/// WebCodecs probe matrix at app startup so the backend can run
/// `negotiate_session_config` with up-to-date local caps. Subsequent
/// `CommunityEvent::VideoSessionConfig` emissions then reflect the
/// real local encoder + decoder reach instead of the conservative
/// `MediaCapabilities::interim_default()` placeholder.
#[tauri::command]
pub async fn report_local_video_capabilities(
    state: State<'_, SharedState>,
    caps: rekindle_video::MediaCapabilities,
) -> Result<(), String> {
    crate::services::community::video_session::on_local_caps_reported(state.inner(), caps)
}

/// Phase 2 — frontend reports encoder lifecycle events (configure
/// success/failure, watchdog recreations, fatal stops). Mirror of
/// `report_video_decoder_status` for the SEND side: the WKWebView /
/// WebKitGTK encoder divergence must be visible in `RUST_LOG` traces.
/// `community_id` is set for community calls, `peer_id` for DM calls.
#[tauri::command]
pub async fn report_video_encoder_status(
    community_id: Option<String>,
    peer_id: Option<String>,
    codec: String,
    ok: bool,
    detail: Option<String>,
) -> Result<(), String> {
    let scope = community_id
        .or(peer_id)
        .unwrap_or_else(|| "<unscoped>".to_string());
    if ok {
        tracing::info!(
            target: "rekindle_video::encoder",
            scope = %scope,
            codec = %codec,
            detail = detail.as_deref().unwrap_or(""),
            "encoder ok"
        );
    } else {
        tracing::warn!(
            target: "rekindle_video::encoder",
            scope = %scope,
            codec = %codec,
            detail = detail.as_deref().unwrap_or("<unspecified>"),
            "encoder failure"
        );
    }
    Ok(())
}

/// Phase F — frontend reports the result of its `decoder.configure()`
/// call. Backend logs structurally so the WKWebView / WebKitGTK divergence
/// in receiver decode is visible in trace logs without UI screenshots.
/// Frontend capture failures (getUserMedia / getDisplayMedia) must be
/// visible in the terminal log — the webview console is invisible in
/// normal dev runs, and a silent camera failure reads as "the camera
/// just doesn't open".
#[tauri::command]
pub async fn report_media_capture_error(
    stage: String,
    message: String,
) -> Result<(), String> {
    // `*-settings` stages are diagnostics (e.g. the camera mode the
    // platform actually delivered vs the requested one), not failures.
    if stage.ends_with("-settings") {
        tracing::info!(
            target: "rekindle_video::permissions",
            stage = %stage,
            detail = %message,
            "frontend media capture report"
        );
    } else {
        tracing::warn!(
            target: "rekindle_video::permissions",
            stage = %stage,
            error = %message,
            "frontend media capture failure"
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn report_video_decoder_status(
    community_id: String,
    sender_pseudonym: String,
    stream_id: String,
    ok: bool,
    error_message: Option<String>,
) -> Result<(), String> {
    if ok {
        tracing::info!(
            target: "rekindle_video::decoder",
            community_id = %community_id,
            sender_pseudonym = %sender_pseudonym,
            stream_id = %stream_id,
            "decoder.configure() ok"
        );
    } else {
        tracing::warn!(
            target: "rekindle_video::decoder",
            community_id = %community_id,
            sender_pseudonym = %sender_pseudonym,
            stream_id = %stream_id,
            error = error_message.as_deref().unwrap_or("<unspecified>"),
            "decoder.configure() failed"
        );
    }
    Ok(())
}
