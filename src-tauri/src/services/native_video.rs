//! Facade over the native camera capture+encode pipeline
//! (`rekindle-video-capture`). The crate now exposes ONE uniform public
//! surface on every platform — `capture_available()`, `list_devices()`,
//! and a `NativeCaptureSession` with `start`/`stop`/`set_bitrate_kbps`/
//! `force_keyframe` producing `EncodedFrame` (peer egress) and
//! `PreviewFrame` (self-view) — so this facade no longer platform-branches
//! itself. Capture is GStreamer on every platform (`v4l2src`/`pipewiresrc`
//! on Linux, `avfvideosrc` on macOS, `ksvideosrc`/`mfvideosrc` on Windows,
//! chosen by the OS's GStreamer device monitor), so there is no per-OS Rust
//! capture code; the nokhwa backend was retired. Callers
//! (commands, adapters, teardown) ask `capture_available` instead of
//! sniffing the OS.
//!
//! The pump task owns the capture session. One capture, fanned out: the
//! ENCODE branch converges on the same egress as the webview path
//! (`send_encoded_video_frame` → media-ready gate → MEK encrypt →
//! fragment → pacer); the PREVIEW branch emits small JPEG stills the pump
//! forwards to the webview's self-view channel (painted to a canvas via
//! `createImageBitmap`). No second `getUserMedia` consumer of the camera,
//! no encode→decode loopback for local pixels — the single-capture +
//! in-process fan-out every native P2P client uses.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rekindle_video_capture::{CaptureConfig, NativeCaptureSession};

use crate::state::AppState;

/// Serializable device entry for the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeVideoDevice {
    pub display_name: String,
}

/// Encode shape for the native path — the negotiated §10.6 interim
/// ceiling. Resolution is fixed for the stream's life (decoders break
/// on in-band resize); rate adaptation is bitrate-only, which the VP9
/// encoder actually honors.
const NATIVE_WIDTH: u32 = 854;
const NATIVE_HEIGHT: u32 = 480;
const NATIVE_FPS: u32 = 15;
/// Encoder-internal keyframe ceiling in frames (4 s at 15 fps) — the
/// keyframe-request path forces earlier ones on demand.
const NATIVE_KEYFRAME_MAX_DIST: u32 = 60;

/// Sender-side keyframe floor — mirrors the frontend
/// `KEYFRAME_MIN_INTERVAL_MS` (libwebrtc's 300 ms).
const FORCE_KEYFRAME_FLOOR: Duration = Duration::from_millis(300);

enum Control {
    Stop,
    ForceKeyframe,
    /// The allocator's encoder target, kbps (plan E4.3.3).
    Bitrate(u32),
    /// The user chose another camera (`None` = the first device).
    SwitchDevice(Option<String>),
}

struct ActiveSession {
    stream_id: [u8; 16],
    stream_id_hex: String,
    control_tx: tokio::sync::mpsc::Sender<Control>,
}

/// Slot state stored in `AppState` on every platform, so state
/// construction never platform-branches. Empty until a session starts.
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

/// Codecs the native path can encode — unioned into the local
/// `MediaCapabilities` so codec negotiation can pick VP9 without
/// the webview probe knowing about the native encoder.
pub fn native_encode_codecs() -> Vec<rekindle_types::video::Codec> {
    if capture_available() {
        vec![rekindle_types::video::Codec::Vp9]
    } else {
        Vec::new()
    }
}

/// The active native stream id (hex), when a session is running —
/// the frontend's pre-emption guard (a busy camera surfaces NO error
/// through getUserMedia, so webview consumers must ask first).
pub fn active_stream_id(state: &AppState) -> Option<String> {
    state
        .native_video
        .active
        .lock()
        .as_ref()
        .map(|s| s.stream_id_hex.clone())
}

/// Stop the active native session (no-op when none).
pub fn stop(state: &AppState) {
    if let Some(session) = state.native_video.active.lock().take() {
        let _ = session.control_tx.try_send(Control::Stop);
    }
}

/// FIR semantics: force a keyframe on the active native stream.
pub fn force_keyframes(state: &AppState) {
    if let Some(session) = state.native_video.active.lock().as_ref() {
        let _ = session.control_tx.try_send(Control::ForceKeyframe);
    }
}

/// A `KeyframeRequest` arrived for `stream_id` — returns true when it
/// was handled by the native encoder (the webview sender then has
/// nothing to do).
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

/// The allocator moved the video encoder target (plan E4.3.3).
pub fn set_target_kbps(state: &AppState, kbps: u32) {
    if let Some(session) = state.native_video.active.lock().as_ref() {
        let _ = session.control_tx.try_send(Control::Bitrate(kbps));
    }
}

/// Move the running camera session to `device_label` (the user chose
/// another camera mid-call). The capture is reopened under the same stream
/// id and frame sequence, the way `RTCRtpSender.replaceTrack` swaps a
/// sender's source without renegotiation (W3C webrtc-pc §5.2); the new
/// encoder's first frame is a keyframe. No-op when no session runs: the
/// next start reads the saved choice.
pub fn switch_device(state: &AppState, device_label: Option<String>) {
    if let Some(session) = state.native_video.active.lock().as_ref() {
        let _ = session
            .control_tx
            .try_send(Control::SwitchDevice(device_label));
    }
}

/// Save the camera choice: the device id the webview path resolves first,
/// and the label both paths share.
pub fn persist_video_device_prefs(
    app: &tauri::AppHandle,
    device_id: Option<String>,
    device_label: Option<String>,
) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;

    let store = app.store("preferences.json").map_err(|e| e.to_string())?;
    let mut prefs: crate::commands::settings::Preferences = store
        .get("preferences")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    prefs.video_device_id = device_id;
    prefs.video_device_label = device_label;
    let val = serde_json::to_value(&prefs).map_err(|e| e.to_string())?;
    store.set("preferences", val);
    store.save().map_err(|e| e.to_string())
}

/// Stop `session` and open the camera again with `config`: stop first,
/// since a capture device may allow only one opener (V4L2). The new
/// pipeline gets its own error channel, so a late error from the stopped
/// one cannot end it.
async fn reopen(
    session: NativeCaptureSession,
    config: CaptureConfig,
    frame_tx: tokio::sync::mpsc::Sender<rekindle_video_capture::EncodedFrame>,
    preview_tx: tokio::sync::mpsc::Sender<rekindle_video_capture::PreviewFrame>,
) -> Result<(NativeCaptureSession, tokio::sync::mpsc::Receiver<String>), String> {
    session.stop();
    let (error_tx, error_rx) = tokio::sync::mpsc::channel(4);
    let session = tokio::task::spawn_blocking(move || {
        NativeCaptureSession::start(&config, frame_tx, preview_tx, error_tx)
    })
    .await
    .map_err(|e| format!("capture start task: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok((session, error_rx))
}

/// The session ended on an error: release the slot and tell the call
/// window, which reverts the camera toggle and shows `message`.
fn fail_session(
    state: &AppState,
    app: &tauri::AppHandle,
    community_id: &str,
    channel_id: &str,
    message: String,
) {
    tracing::warn!(
        target: "rekindle_video_capture",
        community_id = %community_id,
        %message,
        "native capture failed — stopping session"
    );
    state.native_video.active.lock().take();
    crate::event_dispatch::emit_community(
        app,
        crate::channels::CommunityEvent::NativeVideoError(crate::channels::NativeVideoErrorEvent {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message,
        }),
    );
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
    if state.native_video.active.lock().is_some() {
        return Err("camera-session-active".into());
    }
    // nokhwa's AVCaptureDevice open does not reliably trigger the macOS
    // TCC prompt on its own; front-load the request (idempotent — a
    // status precheck) before the capture thread opens the device.
    #[cfg(target_os = "macos")]
    crate::platform::request_camera_authorization();

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

    // Derive the capture + encode target from the room's negotiated encoder
    // ceiling (gap item 2) so native capture follows a lifted ceiling or a
    // weak-peer downgrade; fall back to the interim consts before the first
    // negotiation has produced a config. The scorer treats this as its match
    // target and `scale.rs` downscales native frames to it.
    let (target_width, target_height, target_fps) = state
        .video_sessions
        .last_config(community_id, channel_id)
        .map_or((NATIVE_WIDTH, NATIVE_HEIGHT, NATIVE_FPS), |cfg| {
            (
                cfg.encoder.max_width,
                cfg.encoder.max_height,
                cfg.encoder.max_fps,
            )
        });

    let config = CaptureConfig {
        device_label,
        source_override: None,
        width: target_width,
        height: target_height,
        fps: target_fps,
        start_bitrate_kbps: crate::services::voice_adapter::video_allocation::encoder_kbps(state),
        keyframe_max_dist: NATIVE_KEYFRAME_MAX_DIST,
    };
    // start() blocks up to its 2 s first-sample deadline. The pump keeps
    // the senders, so a device switch reopens onto the same channels.
    let session = tokio::task::spawn_blocking({
        let config = config.clone();
        let (frame_tx, preview_tx) = (frame_tx.clone(), preview_tx.clone());
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

    // The pump: frames out, preview out, control (stop, keyframe,
    // bitrate) in.
    let pump_state = Arc::clone(state);
    let pump_app = app.clone();
    let pump_community = community_id.to_string();
    let pump_channel = channel_id.to_string();
    let pump_stream_hex = stream_id_hex.clone();
    crate::state_helpers::spawn_in_login_with_token(
        state,
        "native video pump",
        |stop| async move {
            let mut session = session;
            let mut config = config;
            let mut frame_seq: u32 = 0;
            let mut preview_count: u64 = 0;
            let mut last_forced = Instant::now();
            loop {
                tokio::select! {
                    biased;
                    // The session ended: release the camera like a Stop.
                    () = stop.cancelled() => {
                        pump_state.native_video.active.lock().take();
                        session.stop();
                        break;
                    }
                    control = control_rx.recv() => {
                        match control {
                            Some(Control::ForceKeyframe) => {
                                if last_forced.elapsed() >= FORCE_KEYFRAME_FLOOR {
                                    last_forced = Instant::now();
                                    session.force_keyframe();
                                }
                            }
                            Some(Control::Bitrate(kbps)) => {
                                config.start_bitrate_kbps = kbps;
                                session.set_bitrate_kbps(kbps);
                            }
                            Some(Control::SwitchDevice(device_label)) => {
                                tracing::info!(
                                    target: "rekindle_video_capture",
                                    community_id = %pump_community,
                                    device = ?device_label,
                                    "switching camera"
                                );
                                config.device_label = device_label;
                                let reopened = reopen(
                                    session,
                                    config.clone(),
                                    frame_tx.clone(),
                                    preview_tx.clone(),
                                )
                                .await;
                                match reopened {
                                    Ok((next, next_errors)) => {
                                        session = next;
                                        error_rx = next_errors;
                                    }
                                    Err(message) => {
                                        fail_session(
                                            &pump_state,
                                            &pump_app,
                                            &pump_community,
                                            &pump_channel,
                                            message,
                                        );
                                        break;
                                    }
                                }
                            }
                            Some(Control::Stop) | None => {
                                session.stop();
                                break;
                            }
                        }
                    }
                    error = error_rx.recv() => {
                        let message = error.unwrap_or_else(|| "camera pipeline ended".into());
                        session.stop();
                        fail_session(
                            &pump_state,
                            &pump_app,
                            &pump_community,
                            &pump_channel,
                            message,
                        );
                        break;
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
                    }
                    frame = frame_rx.recv() => {
                        let Some(frame) = frame else {
                            // Pipeline torn down — sender side dropped.
                            pump_state.native_video.active.lock().take();
                            break;
                        };
                        frame_seq = frame_seq.wrapping_add(1);
                        // Wire timestamp = capture time on the wall clock, ms mod
                        // 2^32 (the field is u32), as the webview sender
                        // stamps it: receivers difference it for jitter and
                        // compare it with the audio's capture stamps for
                        // lip sync.
                        let wire_ts =
                            u32::try_from(frame.capture_wall_ms & u64::from(u32::MAX))
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
            }
            tracing::info!(
                target: "rekindle_video_capture",
                community_id = %pump_community,
                "native capture pump ended"
            );
        },
    );

    Ok(stream_id_hex)
}
