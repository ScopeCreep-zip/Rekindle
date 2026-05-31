//! Phase 16 — Community video adapter.
//!
//! Implements `rekindle_video::VideoDeps` against the live `AppState`
//! + `tauri::AppHandle`. The crate's send + receive pipelines
//! parameterise over this trait so the protocol logic stays free of
//! Tauri/Veilid concerns (Invariant 2).

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_video::{VideoDeps, VideoEvent};

use crate::channels::CommunityEvent;
use crate::state::AppState;
use crate::state_helpers;

pub struct VideoAdapter {
    pub(crate) state: Arc<AppState>,
    pub(crate) app_handle: tauri::AppHandle,
}

impl VideoAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { state, app_handle })
    }
}

impl VideoDeps for VideoAdapter {
    fn community_mek_bytes(&self, community_id: &str) -> Option<([u8; 32], u64)> {
        self.state
            .mek_cache
            .lock()
            .get(community_id)
            .map(|m| (*m.as_bytes(), m.generation()))
    }

    fn community_signing_key(&self, community_id: &str) -> Option<SigningKey> {
        let secret = (*self.state.identity_secret.lock())?;
        Some(rekindle_crypto::group::pseudonym::derive_community_pseudonym(
            &secret,
            community_id,
        ))
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), rekindle_video::VideoError> {
        crate::services::community::send_to_mesh(&self.state, community_id, envelope)
            .map_err(rekindle_video::VideoError::Transport)
    }

    fn increment_lamport(&self, community_id: &str) -> u64 {
        state_helpers::increment_lamport(&self.state, community_id)
    }

    fn emit_event(&self, event: VideoEvent) {
        let mapped = map_video_event(event);
        crate::event_dispatch::emit_live(&self.app_handle, "community-event", &mapped);
    }
}

fn map_video_event(event: VideoEvent) -> CommunityEvent {
    use base64::Engine as _;
    match event {
        VideoEvent::FrameReady {
            community_id,
            sender_pseudonym,
            stream_id,
            frame_seq,
            keyframe,
            timestamp,
            payload,
        } => CommunityEvent::VideoFrame {
            community_id,
            sender_pseudonym,
            stream_id: hex::encode(stream_id),
            frame_seq,
            keyframe,
            timestamp,
            payload_b64: base64::engine::general_purpose::STANDARD.encode(&payload),
        },
        VideoEvent::FrameAck {
            community_id,
            sender_pseudonym,
            channel_id,
            stream_id,
            last_frame_seq,
            kbps,
            loss_q8,
        } => CommunityEvent::VideoFrameAck {
            community_id,
            sender_pseudonym,
            channel_id,
            stream_id: hex::encode(stream_id),
            last_frame_seq,
            kbps,
            loss_q8,
        },
        VideoEvent::KeyframeRequest {
            community_id,
            sender_pseudonym,
            channel_id,
            stream_id,
        } => CommunityEvent::VideoKeyframeRequest {
            community_id,
            sender_pseudonym,
            channel_id,
            stream_id: hex::encode(stream_id),
        },
        VideoEvent::BandwidthEstimate {
            community_id,
            sender_pseudonym,
            channel_id,
            kbps,
            window_secs,
            loss_q8,
        } => CommunityEvent::VideoBandwidthEstimate {
            community_id,
            sender_pseudonym,
            channel_id,
            kbps,
            window_secs,
            loss_q8,
        },
        VideoEvent::TopologyChange {
            community_id,
            sender_pseudonym,
            channel_id,
            stream_id,
            relay_host_pseudonym,
            reason,
            lamport,
        } => CommunityEvent::VideoTopologyChange {
            community_id,
            sender_pseudonym,
            channel_id,
            stream_id: hex::encode(stream_id),
            relay_host_pseudonym,
            reason,
            lamport,
        },
        VideoEvent::MediaCapabilities {
            community_id,
            sender_pseudonym,
            channel_id,
            max_pixel_count,
            max_fps,
            codecs,
        } => CommunityEvent::VideoMediaCapabilities {
            community_id,
            sender_pseudonym,
            channel_id,
            max_pixel_count,
            max_fps,
            codecs,
        },
    }
}

// ── Free-fn facades (preserve pre-Phase-16 signatures) ───────────────

/// Send a video frame to the community mesh. Builds a VideoAdapter +
/// delegates to `rekindle_video::send_video_frame`.
pub fn send_video_frame(
    state: &crate::state::SharedState,
    community_id: &str,
    channel_id: &str,
    request: &rekindle_video::VideoFrameSend,
) -> Result<u32, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle not initialized".to_string())?;
    let adapter = VideoAdapter::new(state.clone(), app_handle);
    rekindle_video::send_video_frame(
        adapter.as_ref(),
        &state.video_reassembly,
        community_id,
        channel_id,
        request,
    )
    .map_err(|e| e.to_string())
}

/// Receive-side dispatcher facade. Builds a VideoAdapter + delegates
/// to `rekindle_video::handle_video_payload`.
pub fn handle_video_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    let adapter = VideoAdapter::new(state.clone(), app_handle.clone());
    let now_ms = u32::try_from(rekindle_utils::timestamp_ms() % u64::from(u32::MAX)).unwrap_or(0);
    rekindle_video::handle_video_payload(
        adapter.as_ref(),
        &state.video_reassembly,
        community_id,
        sender_pseudonym,
        payload,
        now_ms,
    );
}
