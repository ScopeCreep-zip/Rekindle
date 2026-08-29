//! Cross-platform facade over the Linux-native camera capture+encode
//! pipeline (`rekindle-video-capture`). Follows the `idle_service.rs`
//! house pattern: a cfg-gated platform module plus same-signature
//! stubs, so callers (commands, adapters, teardown) never platform-
//! branch themselves — the frontend asks `capture_available` instead
//! of sniffing the OS.
//!
//! The pump task owns the GStreamer session. One capture (`v4l2src`),
//! `tee`-fanned: the ENCODE branch converges on the same egress as the
//! webview path (`send_encoded_video_frame` → media-ready gate → MEK
//! encrypt → fragment → pacer); the PREVIEW branch emits small JPEG
//! stills the pump forwards to the webview's self-view channel (painted
//! to a canvas via `createImageBitmap`). No second `getUserMedia`
//! consumer of the camera, no encode→decode loopback for local pixels —
//! the single-capture + in-process fan-out every native P2P client uses.

use std::sync::Arc;

use crate::state::AppState;

/// Serializable device entry for the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeVideoDevice {
    pub display_name: String,
}

/// Encode shape for the native path — the negotiated §10.6 interim
/// ceiling. Resolution is fixed for the stream's life (decoders break
/// on in-band resize); rate adaptation is bitrate-only, which vp9enc
/// actually honors.
#[cfg(target_os = "linux")]
const NATIVE_WIDTH: u32 = 854;
#[cfg(target_os = "linux")]
const NATIVE_HEIGHT: u32 = 480;
#[cfg(target_os = "linux")]
const NATIVE_FPS: u32 = 15;
/// Encoder-internal keyframe ceiling in frames (4 s at 15 fps) — the
/// keyframe-request path forces earlier ones on demand.
#[cfg(target_os = "linux")]
const NATIVE_KEYFRAME_MAX_DIST: u32 = 60;

pub fn capture_available() -> bool {
    platform::capture_available()
}

pub fn list_devices() -> Vec<NativeVideoDevice> {
    platform::list_devices()
}

/// Start the native camera for the given voice channel. Returns the
/// stream id (hex) on success. Errors are user-displayable strings
/// (`camera busy: …`, `camera-session-active`, …).
pub async fn start(
    state: &Arc<AppState>,
    app: &tauri::AppHandle,
    community_id: &str,
    channel_id: &str,
    track_label: &str,
    device_label: Option<String>,
) -> Result<String, String> {
    platform::start(
        state,
        app,
        community_id,
        channel_id,
        track_label,
        device_label,
    )
    .await
}

/// Stop the active native session (no-op when none).
pub fn stop(state: &AppState) {
    platform::stop(state);
}

/// The active native stream id (hex), when a session is running —
/// the frontend's pre-emption guard (a busy camera surfaces NO error
/// through getUserMedia, so webview consumers must ask first).
pub fn active_stream_id(state: &AppState) -> Option<String> {
    platform::active_stream_id(state)
}

/// FIR semantics: force a keyframe on the active native stream.
pub fn force_keyframes(state: &AppState) {
    platform::force_keyframes(state);
}

/// A `KeyframeRequest` arrived for `stream_id` — returns true when it
/// was handled by the native encoder (the webview sender then has
/// nothing to do).
pub fn on_keyframe_request(state: &AppState, stream_id: &[u8; 16]) -> bool {
    platform::on_keyframe_request(state, stream_id)
}

/// Codecs the native path can encode — unioned into the local
/// `MediaCapabilities` so codec negotiation can pick VP9 without
/// the webview probe knowing about the native encoder.
pub fn native_encode_codecs() -> Vec<rekindle_types::video::Codec> {
    platform::native_encode_codecs()
}

/// Slot state stored in `AppState` (both platforms — a unit off-Linux
/// so `AppState::default()` stays platform-free).
pub use platform::NativeVideoSlot;

#[cfg(target_os = "linux")]
mod platform {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use parking_lot::Mutex;
    use rekindle_video_capture::{CaptureConfig, NativeCaptureSession};

    use super::{
        NativeVideoDevice, NATIVE_FPS, NATIVE_HEIGHT, NATIVE_KEYFRAME_MAX_DIST, NATIVE_WIDTH,
    };
    use crate::state::AppState;

    /// Sender-side keyframe floor — mirrors the frontend
    /// `KEYFRAME_MIN_INTERVAL_MS` (libwebrtc's 300 ms).
    const FORCE_KEYFRAME_FLOOR: Duration = Duration::from_millis(300);

    enum Control {
        Stop,
        ForceKeyframe,
    }

    struct ActiveSession {
        stream_id: [u8; 16],
        stream_id_hex: String,
        control_tx: tokio::sync::mpsc::Sender<Control>,
    }

    #[derive(Default)]
    pub struct NativeVideoSlot {
        active: Mutex<Option<ActiveSession>>,
    }

    pub fn capture_available() -> bool {
        rekindle_video_capture::capture_available()
    }

    pub fn list_devices() -> Vec<NativeVideoDevice> {
        rekindle_video_capture::list_devices()
            .into_iter()
            .map(|d| NativeVideoDevice {
                display_name: d.display_name,
            })
            .collect()
    }

    pub fn native_encode_codecs() -> Vec<rekindle_types::video::Codec> {
        if capture_available() {
            vec![rekindle_types::video::Codec::Vp9]
        } else {
            Vec::new()
        }
    }

    pub fn active_stream_id(state: &AppState) -> Option<String> {
        state
            .native_video
            .active
            .lock()
            .as_ref()
            .map(|s| s.stream_id_hex.clone())
    }

    pub fn stop(state: &AppState) {
        if let Some(session) = state.native_video.active.lock().take() {
            let _ = session.control_tx.try_send(Control::Stop);
        }
    }

    pub fn force_keyframes(state: &AppState) {
        if let Some(session) = state.native_video.active.lock().as_ref() {
            let _ = session.control_tx.try_send(Control::ForceKeyframe);
        }
    }

    pub fn on_keyframe_request(state: &AppState, stream_id: &[u8; 16]) -> bool {
        let guard = state.native_video.active.lock();
        match guard.as_ref() {
            Some(session) if &session.stream_id == stream_id => {
                let _ = session.control_tx.try_send(Control::ForceKeyframe);
                true
            }
            _ => false,
        }
    }

    /// The wire target is in the pacer's WIRE domain; the encoder must
    /// aim at the media-domain conversion or it overproduces into the
    /// pacer queue (R4).
    fn encoder_kbps(state: &AppState, community_id: &str, channel_id: &str) -> u32 {
        let share = state
            .video_payload_share_rx
            .read()
            .as_ref()
            .map_or(rekindle_video::START_PAYLOAD_SHARE_Q10, |rx| *rx.borrow());
        let wire = state
            .video_bitrate_targets
            .lock()
            .get(&(community_id.to_string(), channel_id.to_string()))
            .map_or(rekindle_video::VIDEO_START_KBPS, |(target, _)| *target);
        rekindle_video::encoder_target_kbps(wire, share)
    }

    pub async fn start(
        state: &Arc<AppState>,
        app: &tauri::AppHandle,
        community_id: &str,
        channel_id: &str,
        track_label: &str,
        device_label: Option<String>,
    ) -> Result<String, String> {
        if state.native_video.active.lock().is_some() {
            return Err("camera-session-active".into());
        }
        let stream_id_hex = crate::services::community_video_runtime::derive_video_stream_id_inner(
            state,
            community_id,
            channel_id,
            track_label,
        )?;
        let stream_id: [u8; 16] = hex::decode(&stream_id_hex)
            .map_err(|e| e.to_string())?
            .as_slice()
            .try_into()
            .map_err(|_| "stream id must be 16 bytes".to_string())?;

        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(64);
        let (preview_tx, mut preview_rx) = tokio::sync::mpsc::channel(8);
        let (error_tx, mut error_rx) = tokio::sync::mpsc::channel(4);
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(8);

        let config = CaptureConfig {
            device_label,
            source_override: None,
            width: NATIVE_WIDTH,
            height: NATIVE_HEIGHT,
            fps: NATIVE_FPS,
            start_bitrate_kbps: encoder_kbps(state, community_id, channel_id),
            keyframe_max_dist: NATIVE_KEYFRAME_MAX_DIST,
        };
        // start() blocks up to its 2 s first-sample deadline.
        let session = tokio::task::spawn_blocking({
            let config = config.clone();
            move || NativeCaptureSession::start(&config, frame_tx, preview_tx, error_tx)
        })
        .await
        .map_err(|e| format!("capture start task: {e}"))?
        .map_err(|e| e.to_string())?;

        *state.native_video.active.lock() = Some(ActiveSession {
            stream_id,
            stream_id_hex: stream_id_hex.clone(),
            control_tx,
        });

        // The pump: frames out, preview out, control in, bitrate
        // follow-the-watch.
        let pump_state = Arc::clone(state);
        let pump_app = app.clone();
        let pump_community = community_id.to_string();
        let pump_channel = channel_id.to_string();
        let pump_stream_hex = stream_id_hex.clone();
        tokio::spawn(async move {
            let mut frame_seq: u32 = 0;
            let mut preview_count: u64 = 0;
            let mut last_forced = Instant::now();
            let mut rate_rx = pump_state
                .video_pacer_rate_tx
                .read()
                .as_ref()
                .map(tokio::sync::watch::Sender::subscribe);
            let mut share_rx = pump_state.video_payload_share_rx.read().clone();
            loop {
                tokio::select! {
                    biased;
                    control = control_rx.recv() => {
                        match control {
                            Some(Control::ForceKeyframe) => {
                                if last_forced.elapsed() >= FORCE_KEYFRAME_FLOOR {
                                    last_forced = Instant::now();
                                    session.force_keyframe();
                                }
                                continue;
                            }
                            Some(Control::Stop) | None => {
                                session.stop();
                                break;
                            }
                        }
                    }
                    error = error_rx.recv() => {
                        let message = error.unwrap_or_else(|| "camera pipeline ended".into());
                        tracing::warn!(
                            target: "rekindle_video_capture",
                            community_id = %pump_community,
                            %message,
                            "native capture failed — stopping session"
                        );
                        pump_state.native_video.active.lock().take();
                        session.stop();
                        crate::event_dispatch::emit_live(
                            &pump_app,
                            "community-event",
                            &crate::channels::CommunityEvent::NativeVideoError {
                                community_id: pump_community.clone(),
                                channel_id: pump_channel.clone(),
                                message,
                            },
                        );
                        break;
                    }
                    changed = async {
                        match rate_rx.as_mut() {
                            Some(rx) => rx.changed().await.is_ok(),
                            None => std::future::pending().await,
                        }
                    } => {
                        if changed {
                            let wire = rate_rx.as_ref().map_or(0, |rx| *rx.borrow());
                            let share = share_rx.as_ref().map_or(
                                rekindle_video::START_PAYLOAD_SHARE_Q10,
                                |rx| *rx.borrow(),
                            );
                            session.set_bitrate_kbps(rekindle_video::encoder_target_kbps(wire, share));
                        }
                        continue;
                    }
                    preview = preview_rx.recv() => {
                        let Some(preview) = preview else {
                            // Preview branch ended — the encode branch's
                            // own teardown (frame_rx None / error_rx)
                            // owns session lifecycle; just stop forwarding.
                            continue;
                        };
                        // Local self-view: JPEG straight to the webview's
                        // preview channel (canvas paint), never the peer
                        // egress. Best-effort — a dropped still is fine.
                        use base64::Engine;
                        let jpeg_b64 = base64::engine::general_purpose::STANDARD
                            .encode(&preview.jpeg);
                        preview_count += 1;
                        if preview_count == 1 {
                            tracing::info!(
                                target: "rekindle_video_capture",
                                community_id = %pump_community,
                                jpeg_bytes = preview.jpeg.len(),
                                "self-view preview branch producing frames"
                            );
                        }
                        pump_state.video_channels.send_native_preview(
                            crate::video_channels::NativePreviewFrameMsg {
                                stream_id_hex: pump_stream_hex.clone(),
                                jpeg_b64,
                            },
                        );
                        continue;
                    }
                    frame = frame_rx.recv() => {
                        let Some(frame) = frame else {
                            // Pipeline torn down — sender side dropped.
                            pump_state.native_video.active.lock().take();
                            break;
                        };
                        frame_seq = frame_seq.wrapping_add(1);
                        // Wire timestamp = unix ms (u32-wrapped, same
                        // modulus as the receive path's now_ms) — only
                        // DIFFERENCES matter to receivers' jitter math.
                        let wire_ts = u32::try_from(
                            rekindle_utils::timestamp_ms() % u64::from(u32::MAX),
                        )
                        .unwrap_or(0);
                        // Egress to PEERS only — gate + MEK + fragment +
                        // pacer. No loopback: the local self-view is a
                        // direct getUserMedia preview in the webview
                        // (PipeWire shares the camera), never a decode
                        // round-trip.
                        let _ = crate::services::community_video_runtime::send_encoded_video_frame(
                            &pump_state,
                            &pump_community,
                            &pump_channel,
                            &crate::services::community::video::VideoFrameSend {
                                stream_id,
                                frame_seq,
                                keyframe: frame.keyframe,
                                codec: rekindle_types::video::Codec::Vp9,
                                timestamp: wire_ts,
                                encoded_payload: frame.payload,
                            },
                        );
                    }
                }
                // Share moves rarely; fold it into the rate poll by
                // re-reading on every loop instead of a fifth arm.
                if share_rx.is_none() {
                    share_rx.clone_from(&pump_state.video_payload_share_rx.read());
                }
            }
            tracing::info!(
                target: "rekindle_video_capture",
                community_id = %pump_community,
                "native capture pump ended"
            );
        });

        Ok(stream_id_hex)
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use std::sync::Arc;

    use super::NativeVideoDevice;
    use crate::state::AppState;

    /// Empty slot — `AppState` carries it on every platform so state
    /// construction never platform-branches. Empty-braces (not a unit
    /// struct) so the shared `NativeVideoSlot::default()` call site in
    /// `app_state.rs` doesn't trip `default_constructed_unit_structs`
    /// on this platform, while the Linux build has a real field-bearing
    /// struct.
    #[derive(Default)]
    pub struct NativeVideoSlot {}

    pub fn capture_available() -> bool {
        false
    }

    pub fn list_devices() -> Vec<NativeVideoDevice> {
        Vec::new()
    }

    pub fn native_encode_codecs() -> Vec<rekindle_types::video::Codec> {
        Vec::new()
    }

    pub fn active_stream_id(_state: &AppState) -> Option<String> {
        None
    }

    pub fn stop(_state: &AppState) {}

    pub fn force_keyframes(_state: &AppState) {}

    pub fn on_keyframe_request(_state: &AppState, _stream_id: &[u8; 16]) -> bool {
        false
    }

    #[allow(
        clippy::unused_async,
        reason = "platform stub mirrors the Linux async signature; callers await uniformly"
    )]
    pub async fn start(
        _state: &Arc<AppState>,
        _app: &tauri::AppHandle,
        _community_id: &str,
        _channel_id: &str,
        _track_label: &str,
        _device_label: Option<String>,
    ) -> Result<String, String> {
        Err("native video capture is not available on this platform".into())
    }
}
