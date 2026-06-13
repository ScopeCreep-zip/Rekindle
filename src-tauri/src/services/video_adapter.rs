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

    fn previous_channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<([u8; 32], u64)> {
        crate::state_helpers::previous_channel_mek(&self.state, community_id, channel_id)
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
            // Native-owned streams answer keyframe requests in the
            // backend (force-key-unit into vp8enc); the event still
            // flows to the frontend, where forceKeyframe is a no-op
            // for stream ids the webview sender doesn't own.
            VideoEvent::KeyframeRequest { stream_id, .. } => {
                let _ = crate::services::native_video::on_keyframe_request(&self.state, stream_id);
            }
            _ => {}
        }
        let mapped = map_video_event(event);
        crate::event_dispatch::emit_live(&self.app_handle, "community-event", &mapped);
    }
}

impl VideoAdapter {
    /// One AIMD step from receiver feedback, run in WIRE units (R4 —
    /// the libwebrtc `WithOverhead` model): the receiver can only
    /// count reassembled payload bytes, so its goodput is scaled up by
    /// the pacer's measured payload share before the step; the pacer
    /// watch carries the wire target; the frontend/native encoder gets
    /// the media-domain conversion (`encoder_target_kbps`). Without
    /// the scaling, the GCC growth cap compares wire to payload and
    /// hard-freezes at realistic shares (1.5 × 0.625 < 1.0).
    ///
    /// Policy state and pacer rate advance on EVERY step — a +10% ramp
    /// must compound, and a watch send is free. Only the frontend
    /// `CommunityEvent::VideoBitrateTarget` is gated by the >15%
    /// hysteresis, because the encoder reconfigure it triggers forces
    /// a keyframe.
    fn apply_bitrate_feedback(&self, community_id: &str, channel_id: &str, kbps: u32, loss_q8: u8) {
        let Some(encoder_kbps) =
            bitrate_feedback_step(&self.state, community_id, channel_id, kbps, loss_q8)
        else {
            return;
        };
        tracing::info!(
            target: "rekindle_video::pacer",
            community_id,
            channel_id,
            encoder_kbps,
            feedback_payload_kbps = kbps,
            loss_q8,
            "bitrate target updated"
        );
        let event = CommunityEvent::VideoBitrateTarget {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            // Media-domain rate — what the encoder should PRODUCE so
            // its output fits the wire budget after fragmentation
            // overhead + parity.
            kbps: encoder_kbps,
        };
        crate::event_dispatch::emit_live(&self.app_handle, "community-event", &event);
    }
}

/// The AIMD step minus the event emission, separated so the wire-domain
/// behavior is testable without an `AppHandle`: scales receiver payload
/// goodput to wire units, steps the target, persists policy state,
/// pushes the pacer watch on change — and returns `Some(media-domain
/// encoder target)` only when the >15% emit hysteresis fires.
fn bitrate_feedback_step(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    kbps: u32,
    loss_q8: u8,
) -> Option<u32> {
    let share_q10 = state
        .video_payload_share_rx
        .read()
        .as_ref()
        .map_or(rekindle_video::START_PAYLOAD_SHARE_Q10, |rx| *rx.borrow());
    let feedback_wire = rekindle_video::wire_feedback_kbps(kbps, share_q10);
    let key = (community_id.to_string(), channel_id.to_string());
    let (prev, next, last_emitted) = {
        let mut targets = state.video_bitrate_targets.lock();
        let (prev, emitted) = targets.get(&key).copied().unwrap_or((
            rekindle_video::VIDEO_START_KBPS,
            rekindle_video::VIDEO_START_KBPS,
        ));
        let next = rekindle_video::target_from_feedback(prev, feedback_wire, loss_q8);
        targets.insert(key.clone(), (next, emitted));
        (prev, next, emitted)
    };
    if next != prev {
        if let Some(rate_tx) = state.video_pacer_rate_tx.read().as_ref() {
            let _ = rate_tx.send(next);
        }
    }
    let drift = (f64::from(next) - f64::from(last_emitted)).abs() / f64::from(last_emitted.max(1));
    if drift <= 0.15 {
        return None;
    }
    state.video_bitrate_targets.lock().insert(key, (next, next));
    Some(rekindle_video::encoder_target_kbps(next, share_q10))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// R4 test #7 (adapter integration): the AIMD step runs in WIRE
    /// units. A receiver acking exactly the encoder-domain goodput of
    /// the current target (the death-spiral scenario the payload-unit
    /// loop froze on) must still ramp; the pacer watch carries the
    /// WIRE target; the >15% hysteresis gates ONLY the encoder event,
    /// whose value is the media-domain conversion.
    #[test]
    fn bitrate_feedback_runs_in_wire_domain() {
        let state = Arc::new(crate::state::AppState::default());
        let (_share_tx, share_rx) =
            tokio::sync::watch::channel(rekindle_video::START_PAYLOAD_SHARE_Q10);
        *state.video_payload_share_rx.write() = Some(share_rx);
        let (rate_tx, rate_rx) = tokio::sync::watch::channel(rekindle_video::VIDEO_START_KBPS);
        *state.video_pacer_rate_tx.write() = Some(rate_tx);

        // Clean ack at the encoder-domain goodput of the 350 start
        // target (share 640 → 218 kbps payload).
        let payload = rekindle_video::encoder_target_kbps(350, 640);
        let emitted = bitrate_feedback_step(&state, "c", "ch", payload, 0);
        let key = ("c".to_string(), "ch".to_string());
        let (next, _) = *state.video_bitrate_targets.lock().get(&key).unwrap();
        assert_eq!(next, 385, "wire-domain ramp must not freeze: {next}");
        assert_eq!(*rate_rx.borrow(), 385, "pacer watch carries the WIRE target");
        assert!(
            emitted.is_none(),
            "10% drift is below the 15% emit hysteresis"
        );

        // Second clean ack compounds past the hysteresis → the event
        // fires with the MEDIA-domain value.
        let payload2 = rekindle_video::encoder_target_kbps(385, 640);
        let emitted2 = bitrate_feedback_step(&state, "c", "ch", payload2, 0);
        let (next2, anchored) = *state.video_bitrate_targets.lock().get(&key).unwrap();
        assert_eq!(next2, 423, "ramp compounds across steps");
        assert_eq!(anchored, 423, "emit re-anchors the hysteresis");
        assert_eq!(
            emitted2,
            Some(rekindle_video::encoder_target_kbps(423, 640)),
            "event carries the encoder-domain rate"
        );
    }
}
