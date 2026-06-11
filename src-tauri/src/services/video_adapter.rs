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

    /// Phase F — emit a `VideoEvent::EnvelopeRejected` from outside the
    /// receive pipeline so the protocol-level `verify_envelope` failure
    /// site in `services/veilid/app_message.rs` can surface the drop to
    /// the UI without needing to construct a full `ControlPayload`. The
    /// adapter's normal mapping path (`map_video_event`) handles the
    /// translation to `CommunityEvent::VideoEnvelopeRejected`.
    pub fn emit_video_envelope_rejected(
        &self,
        community_id: String,
        sender_pseudonym: String,
        reason: String,
    ) {
        VideoDeps::emit_event(
            self,
            VideoEvent::EnvelopeRejected {
                community_id,
                sender_pseudonym,
                reason,
            },
        );
    }
}

impl VideoDeps for VideoAdapter {
    fn channel_media_mek(&self, community_id: &str, channel_id: &str) -> Option<([u8; 32], u64)> {
        crate::state_helpers::channel_media_mek(&self.state, community_id, channel_id)
    }

    fn community_signing_key(&self, community_id: &str) -> Option<SigningKey> {
        let secret = (*self.state.identity_secret.lock())?;
        Some(rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id))
    }

    fn send_to_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), rekindle_video::VideoError> {
        crate::services::community::send_to_channel_peers(
            &self.state,
            community_id,
            channel_id,
            envelope,
        )
        .map_err(rekindle_video::VideoError::Transport)
    }

    fn local_active_channel(&self, community_id: &str) -> Option<String> {
        let ve = self.state.voice_engine.lock();
        let handle = ve.as_ref()?;
        (handle.community_id.as_deref() == Some(community_id)).then(|| handle.channel_id.clone())
    }

    fn request_mek_refresh(&self, community_id: &str, channel_id: &str, needed_generation: u64) {
        // Exact-generation request from the undecryptable frame's wire
        // field (0 = "send me your current") — never a guess; the
        // responder can always satisfy it, so recovery converges.
        let Some(my_pseudonym) = self
            .state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
        else {
            return;
        };
        crate::services::community::mek_rotation::spawn_mek_request_with_retry(
            std::sync::Arc::clone(&self.state),
            community_id.to_string(),
            channel_id.to_string(),
            needed_generation,
            my_pseudonym,
        );
    }

    fn increment_lamport(&self, community_id: &str) -> u64 {
        state_helpers::increment_lamport(&self.state, community_id)
    }

    fn emit_event(&self, event: VideoEvent) {
        // Phase 11 Tier 1 — high-throughput reassembled frames bypass the
        // `community-event` bus and go straight to the per-community
        // `ipc::Channel` the video panel registered. The low-rate control
        // events (acks, keyframe requests, topology, capabilities) stay on
        // the event bus.
        if let VideoEvent::FrameReady {
            community_id,
            sender_pseudonym,
            stream_id,
            frame_seq,
            keyframe,
            codec,
            timestamp,
            payload,
        } = event
        {
            use base64::Engine as _;
            self.state.video_channels.send_community(
                &community_id,
                crate::video_channels::CommunityVideoFrameMsg {
                    community_id: community_id.clone(),
                    sender_pseudonym,
                    stream_id: hex::encode(stream_id),
                    frame_seq,
                    keyframe,
                    codec: codec.wire_str().to_string(),
                    timestamp,
                    payload_b64: base64::engine::general_purpose::STANDARD.encode(&payload),
                },
            );
            return;
        }
        // Phase B — feed a gossiped MediaCapabilities into the per-call
        // video session aggregator BEFORE forwarding the raw advertisement
        // to the frontend, so the frontend store sees the policy
        // re-emission (`CommunityEvent::VideoSessionConfig`) in the same
        // event stream as the cap update that produced it. The aggregator
        // emit and the raw cap emit both ride the existing dispatch queue
        // — order is preserved.
        if let VideoEvent::MediaCapabilities {
            community_id,
            sender_pseudonym,
            channel_id,
            max_pixel_count,
            max_fps,
            encode_codecs,
            decode_codecs,
            supports_optimize_for_latency,
            supported_scalability_modes,
        } = &event
        {
            let caps = rekindle_video::MediaCapabilities {
                max_pixel_count: *max_pixel_count,
                max_fps: *max_fps,
                encode_codecs: encode_codecs.clone(),
                decode_codecs: decode_codecs.clone(),
                supports_optimize_for_latency: *supports_optimize_for_latency,
                supported_scalability_modes: supported_scalability_modes.clone(),
            };
            if let Err(e) = crate::services::community::video_session::on_peer_caps_received(
                &self.state,
                community_id,
                channel_id,
                sender_pseudonym,
                caps,
            ) {
                tracing::warn!(error = %e, "video_session::on_peer_caps_received failed");
            }
        }
        // Phase 4 — backend-owned bitrate policy: receiver feedback
        // drives the pacer rate AND a `VideoBitrateTarget` event the
        // frontend encoder follows. Emit hysteresis (>15% move) keeps
        // configure()-forced keyframes rare.
        match &event {
            VideoEvent::FrameAck {
                community_id,
                channel_id,
                kbps,
                loss_q8,
                ..
            }
            | VideoEvent::BandwidthEstimate {
                community_id,
                channel_id,
                kbps,
                loss_q8,
                ..
            } => {
                self.apply_bitrate_feedback(community_id, channel_id, *kbps, *loss_q8);
            }
            _ => {}
        }
        let mapped = map_video_event(event);
        crate::event_dispatch::emit_live(&self.app_handle, "community-event", &mapped);
    }
}

impl VideoAdapter {
    /// One AIMD step from receiver feedback; on a material (>15%) move
    /// update the pacer rate and emit `CommunityEvent::VideoBitrateTarget`.
    fn apply_bitrate_feedback(&self, community_id: &str, channel_id: &str, kbps: u32, loss_q8: u8) {
        let key = (community_id.to_string(), channel_id.to_string());
        let next = {
            let targets = self.state.video_bitrate_targets.lock();
            let prev = targets
                .get(&key)
                .copied()
                .unwrap_or(rekindle_video::VIDEO_START_KBPS);
            rekindle_video::target_from_feedback(prev, kbps, loss_q8)
        };
        let prev_emitted = self
            .state
            .video_bitrate_targets
            .lock()
            .get(&key)
            .copied()
            .unwrap_or(rekindle_video::VIDEO_START_KBPS);
        let drift =
            (f64::from(next) - f64::from(prev_emitted)).abs() / f64::from(prev_emitted.max(1));
        if drift <= 0.15 {
            return;
        }
        self.state.video_bitrate_targets.lock().insert(key, next);
        if let Some(rate_tx) = self.state.video_pacer_rate_tx.read().as_ref() {
            let _ = rate_tx.send(next);
        }
        tracing::info!(
            target: "rekindle_video::pacer",
            community_id,
            channel_id,
            kbps = next,
            feedback_kbps = kbps,
            loss_q8,
            "bitrate target updated"
        );
        let event = CommunityEvent::VideoBitrateTarget {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            kbps: next,
        };
        crate::event_dispatch::emit_live(&self.app_handle, "community-event", &event);
    }
}

fn map_video_event(event: VideoEvent) -> CommunityEvent {
    match event {
        // Phase 11 Tier 1 — frames are routed to the per-community
        // `ipc::Channel` in `emit_event` before this mapper runs, so a
        // `FrameReady` here means the dispatch invariant was violated.
        VideoEvent::FrameReady { .. } => {
            unreachable!("FrameReady is forwarded to the video ipc::Channel in emit_event")
        }
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
            encode_codecs,
            decode_codecs,
            supports_optimize_for_latency,
            supported_scalability_modes,
        } => CommunityEvent::VideoMediaCapabilities {
            community_id,
            sender_pseudonym,
            channel_id,
            max_pixel_count,
            max_fps,
            encode_codecs,
            decode_codecs,
            supports_optimize_for_latency,
            supported_scalability_modes,
        },
        // Phase F — surface the asymmetric-drop case to the UI so
        // sender-side and receiver-side observers both see the same
        // verification failure. The frontend listens on
        // `community-event` and shows a toast / status pill instead of
        // requiring a grep through Rust logs to detect the condition.
        VideoEvent::EnvelopeRejected {
            community_id,
            sender_pseudonym,
            reason,
        } => CommunityEvent::VideoEnvelopeRejected {
            community_id,
            sender_pseudonym,
            reason,
        },
    }
}

// ── Free-fn facades (preserve pre-Phase-16 signatures) ───────────────

/// Build a video frame (MEK-encrypt + fragment + sign) and hand it to
/// the per-session pacer, which releases fragments at the audio-first
/// budgeted rate. The frame count returned is the fragment count the
/// pacer will release.
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
    let frame = rekindle_video::build_video_frame(
        adapter.as_ref(),
        &state.video_reassembly,
        community_id,
        channel_id,
        request,
        rekindle_utils::timestamp_ms(),
    )
    .map_err(|e| e.to_string())?;
    let fragment_count = u32::try_from(frame.envelopes.len()).unwrap_or(u32::MAX);
    let pacer_tx = state.video_pacer_tx.read().clone();
    let Some(tx) = pacer_tx else {
        return Err("video pacer not running — no active voice session".to_string());
    };
    if tx.try_send(frame).is_err() {
        let n = state
            .video_pacer_send_drops
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if n == 1 || n.is_multiple_of(30) {
            tracing::warn!(
                target: "rekindle_video::pacer",
                community_id,
                channel_id,
                dropped_total = n,
                "video pacer saturated — frame refused at intake"
            );
        }
        return Err("video pacer saturated".to_string());
    }
    Ok(fragment_count)
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
