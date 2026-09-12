//! The native capture session: a dedicated OS thread owns the nokhwa
//! camera and the libvpx encoder (both stay thread-local, the `cpal`
//! posture `rekindle-voice` uses for its !Send audio streams), converts
//! each frame to I420, scales it to the encoder input, encodes VP9 CBR, and
//! emits `EncodedFrame`s to the peer egress plus small JPEG `PreviewFrame`s
//! to the self-view. Control (bitrate follow, keyframe request, stop)
//! crosses to the thread through lock-free atomics, so the session handle
//! itself is trivially `Send + Sync` and stores no nokhwa objects.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::mpsc;

use rekindle_video::codec::VideoEncoder;
use rekindle_video::{Codec, EncoderConstraints, ScalabilityMode, VideoError};
use rekindle_video_libvpx::LibvpxEncoder;

use super::{camera, convert, preview, scale};
use crate::shared::{CaptureConfig, CaptureError, EncodedFrame, PreviewFrame};

/// Self-view thumbnail geometry (mirrors the GStreamer preview branch).
const PREVIEW_WIDTH: u32 = 320;
const PREVIEW_HEIGHT: u32 = 180;
const PREVIEW_QUALITY: u8 = 50;

/// How long `start` waits for the first encoded frame before declaring the
/// camera dead (mirrors the GStreamer path's 2 s first-sample deadline).
const START_DEADLINE: Duration = Duration::from_secs(2);

/// Lock-free control channel between the session handle and its capture
/// thread. `bitrate_kbps` is the live encoder target; `force_kf` requests
/// the next frame be a keyframe; `stop` tears the thread down.
struct Control {
    bitrate_kbps: AtomicU32,
    force_kf: AtomicBool,
    stop: AtomicBool,
}

/// A running native capture session. Dropping it — or calling `stop` —
/// stops the capture thread.
pub struct NativeCaptureSession {
    control: Arc<Control>,
    join: Option<JoinHandle<()>>,
    source_desc: String,
}

impl NativeCaptureSession {
    /// Start capture+encode and block until the first encoded frame
    /// confirms the camera is live (success), or a setup error / the start
    /// deadline (failure). Blocking — callers run it on a blocking thread,
    /// exactly as the GStreamer path is driven.
    pub fn start(
        config: &CaptureConfig,
        frame_tx: mpsc::Sender<EncodedFrame>,
        preview_tx: mpsc::Sender<PreviewFrame>,
        error_tx: mpsc::Sender<String>,
    ) -> Result<Self, CaptureError> {
        let control = Arc::new(Control {
            bitrate_kbps: AtomicU32::new(config.start_bitrate_kbps.max(1)),
            force_kf: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<String, CaptureError>>();
        let loop_cfg = config.clone();
        let loop_ctl = Arc::clone(&control);
        let join = std::thread::Builder::new()
            .name("native-video-capture".into())
            .spawn(move || {
                capture_loop(
                    &loop_cfg,
                    &loop_ctl,
                    &ready_tx,
                    &frame_tx,
                    &preview_tx,
                    &error_tx,
                );
            })
            .map_err(|e| CaptureError::Pipeline(format!("capture thread spawn: {e}")))?;

        match ready_rx.recv_timeout(START_DEADLINE + Duration::from_millis(500)) {
            Ok(Ok(source_desc)) => Ok(Self {
                control,
                join: Some(join),
                source_desc,
            }),
            // The thread reported a setup failure and is exiting on its own.
            Ok(Err(e)) => Err(e),
            // Deadline (or the thread vanished): signal stop and detach —
            // never block the caller joining a thread that may still be in a
            // blocking camera read.
            Err(_) => {
                control.stop.store(true, Ordering::Release);
                Err(CaptureError::Timeout(
                    "camera produced no frame within the start deadline".into(),
                ))
            }
        }
    }

    /// Follow the bitrate policy's encoder-domain target (applied by the
    /// capture thread on its next frame — a live CBR retarget, no rebuild).
    pub fn set_bitrate_kbps(&self, kbps: u32) {
        self.control
            .bitrate_kbps
            .store(kbps.max(1), Ordering::Release);
    }

    /// PLI analog: force the next encoded frame to be a keyframe. Callers
    /// throttle (the existing 300 ms keyframe floor).
    pub fn force_keyframe(&self) {
        self.control.force_kf.store(true, Ordering::Release);
    }

    /// Explicit, logged teardown. Signals the thread and joins it (the
    /// thread is actively producing frames past start, so it observes the
    /// stop flag within one frame interval).
    pub fn stop(mut self) {
        self.control.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        tracing::info!(
            target: "rekindle_video_capture",
            source = %self.source_desc,
            "native capture session stopped"
        );
    }
}

impl Drop for NativeCaptureSession {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Release);
        // Detach rather than join: a Drop must not block, and the thread
        // exits on its own once it observes the stop flag.
        drop(self.join.take());
    }
}

/// Build a VP9 realtime CBR encoder configured for the capture target.
fn build_encoder(config: &CaptureConfig) -> Result<LibvpxEncoder, VideoError> {
    let mut encoder = LibvpxEncoder::new(Codec::Vp9)?;
    encoder.configure(&EncoderConstraints {
        codec: Codec::Vp9,
        max_width: config.width,
        max_height: config.height,
        max_fps: config.fps,
        scalability_mode: ScalabilityMode::Flat,
    })?;
    encoder.set_bitrate(config.start_bitrate_kbps.max(1));
    Ok(encoder)
}

/// Convert one camera buffer to I420 at its source resolution. NV12/YUYV
/// are supported directly; any other camera format is rejected (an
/// MJPEG-only camera is not yet handled on the native path).
fn to_i420(buffer: &nokhwa::Buffer) -> Result<convert::I420Buf, CaptureError> {
    use nokhwa::utils::FrameFormat;
    let res = buffer.resolution();
    let (w, h) = (res.width(), res.height());
    match buffer.source_frame_format() {
        FrameFormat::NV12 => convert::nv12_to_i420(buffer.buffer(), w, h),
        FrameFormat::YUYV => convert::yuyv_to_i420(buffer.buffer(), w, h),
        other => Err(CaptureError::Pipeline(format!(
            "camera delivered {other:?}; the native path supports NV12/YUYV only"
        ))),
    }
}

/// The capture thread body: open camera + encoder, then grab → convert →
/// scale → encode → emit until stopped or a stage fails.
fn capture_loop(
    config: &CaptureConfig,
    control: &Arc<Control>,
    ready_tx: &std_mpsc::Sender<Result<String, CaptureError>>,
    frame_tx: &mpsc::Sender<EncodedFrame>,
    preview_tx: &mpsc::Sender<PreviewFrame>,
    error_tx: &mpsc::Sender<String>,
) {
    let (mut camera, source_desc) = match camera::open_camera(config) {
        Ok(pair) => pair,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let mut encoder = match build_encoder(config) {
        Ok(encoder) => encoder,
        Err(e) => {
            let _ = ready_tx.send(Err(CaptureError::Pipeline(format!("encoder init: {e}"))));
            return;
        }
    };

    let mut applied_kbps = config.start_bitrate_kbps.max(1);
    let mut frame_ix: u64 = 0;
    let mut confirmed = false;

    while !control.stop.load(Ordering::Acquire) {
        // Blocking camera read (bounded by the frame interval on a live
        // camera; the stop flag is observed between frames).
        let buffer = match camera.frame() {
            Ok(buffer) => buffer,
            Err(e) => {
                report_stage_error(
                    confirmed,
                    ready_tx,
                    error_tx,
                    CaptureError::Device(format!("camera frame: {e}")),
                );
                break;
            }
        };
        let source = match to_i420(&buffer) {
            Ok(frame) => frame,
            Err(e) => {
                report_stage_error(confirmed, ready_tx, error_tx, e);
                break;
            }
        };
        let scaled = scale::scale_i420(&source, config.width, config.height);

        // Apply pending control: live bitrate retarget + keyframe cadence.
        let want_kbps = control.bitrate_kbps.load(Ordering::Acquire);
        if want_kbps != applied_kbps {
            encoder.set_bitrate(want_kbps);
            applied_kbps = want_kbps;
        }
        let periodic_kf = config.keyframe_max_dist > 0
            && frame_ix.is_multiple_of(u64::from(config.keyframe_max_dist));
        if periodic_kf || control.force_kf.swap(false, Ordering::AcqRel) {
            encoder.force_keyframe();
        }

        // Self-view first (borrows `scaled`), then hand `scaled` to the
        // encoder — no clone of the full frame.
        emit_preview(&scaled, preview_tx);

        match encoder.encode(scaled.into_raw_frame(0)) {
            Ok(Some(encoded)) => {
                if !confirmed {
                    confirmed = true;
                    let _ = ready_tx.send(Ok(source_desc.clone()));
                }
                let _ = frame_tx.try_send(EncodedFrame {
                    payload: encoded.payload,
                    keyframe: encoded.keyframe,
                });
            }
            Ok(None) => {}
            Err(e) => {
                report_stage_error(
                    confirmed,
                    ready_tx,
                    error_tx,
                    CaptureError::Pipeline(format!("encode: {e}")),
                );
                break;
            }
        }
        frame_ix = frame_ix.wrapping_add(1);
    }

    tracing::info!(
        target: "rekindle_video_capture",
        source = %source_desc,
        "native capture loop ended"
    );
}

/// Before the first frame confirms, a failure returns through the start
/// channel; after, through the runtime error channel (session teardown).
fn report_stage_error(
    confirmed: bool,
    ready_tx: &std_mpsc::Sender<Result<String, CaptureError>>,
    error_tx: &mpsc::Sender<String>,
    err: CaptureError,
) {
    if confirmed {
        let _ = error_tx.try_send(err.to_string());
    } else {
        let _ = ready_tx.send(Err(err));
    }
}

/// Downscale the encoder frame to a thumbnail and JPEG it for the self-view
/// (best-effort — a dropped still is fine, JPEG is intra-only).
fn emit_preview(scaled: &convert::I420Buf, preview_tx: &mpsc::Sender<PreviewFrame>) {
    let thumb = scale::scale_i420(scaled, PREVIEW_WIDTH, PREVIEW_HEIGHT);
    match preview::encode_preview_jpeg(&thumb, PREVIEW_QUALITY) {
        Ok(jpeg) => {
            let _ = preview_tx.try_send(PreviewFrame { jpeg });
        }
        Err(e) => {
            tracing::debug!(
                target: "rekindle_video_capture",
                error = %e,
                "self-view preview encode failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(width: u32, height: u32) -> CaptureConfig {
        CaptureConfig {
            device_label: None,
            source_override: None,
            width,
            height,
            fps: 15,
            start_bitrate_kbps: 350,
            keyframe_max_dist: 60,
        }
    }

    #[test]
    fn synthetic_frame_encodes_to_a_nonempty_keyframe() {
        // The convert → scale → encode → EncodedFrame glue the capture loop
        // runs, driven by a synthetic YUYV buffer instead of a live camera:
        // the one stage combination that cannot run headless is the camera
        // grab itself.
        let cfg = test_config(64, 48);
        let mut encoder = build_encoder(&cfg).unwrap();

        let (src_w, src_h) = (96u32, 64u32);
        let mut yuyv = vec![0u8; (src_w * src_h * 2) as usize];
        // A gradient so rate control has real content to compress.
        for (i, byte) in yuyv.iter_mut().enumerate() {
            *byte = u8::try_from(i % 256).unwrap_or(0);
        }
        let i420 = convert::yuyv_to_i420(&yuyv, src_w, src_h).unwrap();
        let scaled = scale::scale_i420(&i420, cfg.width, cfg.height);

        let encoded = encoder
            .encode(scaled.into_raw_frame(0))
            .unwrap()
            .expect("realtime VP9 (lag=0) must emit the first frame");
        let frame = EncodedFrame {
            payload: encoded.payload,
            keyframe: encoded.keyframe,
        };
        assert!(
            !frame.payload.is_empty(),
            "encoded VP9 payload must be non-empty"
        );
        assert!(frame.keyframe, "the first encoded frame must be a keyframe");
    }
}
