//! Tests for [`super::NativeCaptureSession`].

// Tests skip themselves with an eprintln! when GStreamer/the camera is
// unavailable in the build env (CI without /dev/video*); print-stderr is a
// production-code guard, not a test-diagnostic one — keep the skip reason.
#![allow(
    clippy::print_stderr,
    reason = "test-skip diagnostics when GStreamer/camera is unavailable in CI"
)]

use std::time::{Duration, Instant};

use gstreamer as gst;

use crate::error::CaptureError;

use super::*;

fn test_config(bitrate_kbps: u32) -> CaptureConfig {
    CaptureConfig {
        device_label: None,
        source_override: Some("videotestsrc".into()),
        width: 320,
        height: 240,
        fps: 15,
        start_bitrate_kbps: bitrate_kbps,
        keyframe_max_dist: 60,
    }
}

fn drain_for(
    rx: &mut tokio::sync::mpsc::Receiver<EncodedFrame>,
    duration: Duration,
) -> Vec<EncodedFrame> {
    let deadline = Instant::now() + duration;
    let mut frames = Vec::new();
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(f) => frames.push(f),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    frames
}

#[test]
fn frames_flow_and_first_is_keyframe() {
    if !capture_available() {
        eprintln!("skipping: gstreamer unavailable in this environment");
        return;
    }
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(256);
    let (preview_tx, _preview_rx) = tokio::sync::mpsc::channel(64);
    let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
    let session = NativeCaptureSession::start(&test_config(300), frame_tx, preview_tx, error_tx)
        .expect("videotestsrc pipeline starts");
    let frames = drain_for(&mut frame_rx, Duration::from_millis(1_200));
    session.stop();
    assert!(frames.len() >= 5, "got {} frames", frames.len());
    assert!(frames[0].keyframe, "first encoded frame is a keyframe");
}

#[test]
fn force_keyframe_yields_keyframe_quickly() {
    if !capture_available() {
        eprintln!("skipping: gstreamer unavailable in this environment");
        return;
    }
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(256);
    let (preview_tx, _preview_rx) = tokio::sync::mpsc::channel(64);
    let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
    let session = NativeCaptureSession::start(&test_config(300), frame_tx, preview_tx, error_tx)
        .expect("videotestsrc pipeline starts");
    // Let the stream settle past its initial keyframe.
    let _ = drain_for(&mut frame_rx, Duration::from_millis(500));
    session.force_keyframe();
    let after = drain_for(&mut frame_rx, Duration::from_millis(600));
    session.stop();
    // keyframe-max-dist=60 at 15 fps = 4 s natural cadence; the
    // initial keyframe was at t≈0 and the force at t≈0.5 s, so ANY
    // keyframe in this window is attributable to the force. The
    // window may also start with pre-force in-flight deltas.
    let kinds: Vec<bool> = after.iter().map(|f| f.keyframe).collect();
    assert!(
        kinds.contains(&true),
        "keyframe within 600 ms of the force: {kinds:?}"
    );
}

#[test]
fn bitrate_halving_shrinks_output() {
    if !capture_available() {
        eprintln!("skipping: gstreamer unavailable in this environment");
        return;
    }
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(1024);
    let (preview_tx, _preview_rx) = tokio::sync::mpsc::channel(64);
    let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
    let session = NativeCaptureSession::start(&test_config(600), frame_tx, preview_tx, error_tx)
        .expect("videotestsrc pipeline starts");
    let high: usize = drain_for(&mut frame_rx, Duration::from_millis(1_500))
        .iter()
        .map(|f| f.payload.len())
        .sum();
    session.set_bitrate_kbps(120);
    // Settle, then measure.
    let _ = drain_for(&mut frame_rx, Duration::from_millis(500));
    let low: usize = drain_for(&mut frame_rx, Duration::from_millis(1_500))
        .iter()
        .map(|f| f.payload.len())
        .sum();
    session.stop();
    assert!(
        low * 2 < high,
        "120 kbps window ({low} B) should be well under half the 600 kbps window ({high} B)"
    );
}

#[test]
fn post_start_failure_fires_error_channel() {
    // Plan Phase 2 test (d): the ASYNC error path — a finite
    // source ends the stream after start succeeded; the bus
    // watcher must surface it through error_tx (the same path a
    // camera unplug takes).
    if !capture_available() {
        eprintln!("skipping: gstreamer unavailable in this environment");
        return;
    }
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(256);
    let (error_tx, mut error_rx) = tokio::sync::mpsc::channel(4);
    let (preview_tx, _preview_rx) = tokio::sync::mpsc::channel(64);
    let mut config = test_config(300);
    config.source_override = Some("videotestsrc num-buffers=10".into());
    let session = NativeCaptureSession::start(&config, frame_tx, preview_tx, error_tx)
        .expect("finite videotestsrc starts");
    let _ = drain_for(&mut frame_rx, Duration::from_millis(300));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut message = None;
    while Instant::now() < deadline {
        if let Ok(m) = error_rx.try_recv() {
            message = Some(m);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    session.stop();
    let message = message.expect("error channel fires after the stream ends");
    assert!(
        message.contains("ended"),
        "EOS surfaces as a stream-ended error: {message}"
    );
}

#[test]
fn broken_pipeline_reports_unavailable() {
    if gst::init().is_err() {
        return;
    }
    let (frame_tx, _frame_rx) = tokio::sync::mpsc::channel(4);
    let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
    let (preview_tx, _preview_rx) = tokio::sync::mpsc::channel(64);
    let mut config = test_config(300);
    config.source_override = Some("no-such-element-exists".into());
    let err = NativeCaptureSession::start(&config, frame_tx, preview_tx, error_tx).unwrap_err();
    assert!(matches!(err, CaptureError::Unavailable(_)), "{err}");
}
