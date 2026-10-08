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

/// The Mac's Dell WB7022 as `gst-device-monitor-1.0 Video/Source` lists it
/// (avfvideosrc device caps, 2026-10-08).
const DELL_WB7022: &str = "video/x-raw(memory:GLMemory), width=1920, height=1080, format={ (string)UYVY, (string)YUY2 }, framerate={ (fraction)30/1, (fraction)60/1, (fraction)5/1 }, texture-target=rectangle; video/x-raw(memory:GLMemory), width=1280, height=720, format={ (string)UYVY, (string)YUY2 }, framerate={ (fraction)30/1, (fraction)60/1, (fraction)10/1 }, texture-target=rectangle; video/x-raw(memory:GLMemory), width=640, height={ (int)360, (int)480 }, format={ (string)UYVY, (string)YUY2 }, framerate=30/1, texture-target=rectangle; video/x-raw, width=1920, height=1080, format={ (string)UYVY, (string)YUY2, (string)NV12, (string)ARGB, (string)BGRA }, framerate={ (fraction)30/1, (fraction)60/1, (fraction)5/1 }; video/x-raw, width=1280, height=720, format={ (string)UYVY, (string)YUY2, (string)NV12, (string)ARGB, (string)BGRA }, framerate={ (fraction)30/1, (fraction)60/1, (fraction)10/1 }; video/x-raw, width=640, height={ (int)360, (int)480 }, format={ (string)UYVY, (string)YUY2, (string)NV12, (string)ARGB, (string)BGRA }, framerate=30/1";

/// The Linux laptop's Integrated_Webcam_HD (v4l2 device caps, 2026-10-08).
const INTEGRATED_WEBCAM_HD: &str = "video/x-raw, format=YUY2, width=1280, height=720, pixel-aspect-ratio=1/1, framerate=10/1; video/x-raw, format=YUY2, width=640, height=480, pixel-aspect-ratio=1/1, framerate=30/1; video/x-raw, format=YUY2, width=640, height=360, pixel-aspect-ratio=1/1, framerate=30/1; video/x-raw, format=YUY2, width=424, height=240, pixel-aspect-ratio=1/1, framerate=30/1; video/x-raw, format=YUY2, width=320, height=240, pixel-aspect-ratio=1/1, framerate=30/1; video/x-raw, format=YUY2, width=320, height=180, pixel-aspect-ratio=1/1, framerate=30/1; video/x-raw, format=YUY2, width=160, height=120, pixel-aspect-ratio=1/1, framerate=30/1; image/jpeg, parsed=true, width=1280, height=720, pixel-aspect-ratio=1/1, framerate=30/1; image/jpeg, parsed=true, width=960, height=540, pixel-aspect-ratio=1/1, framerate=30/1; image/jpeg, parsed=true, width=848, height=480, pixel-aspect-ratio=1/1, framerate=30/1; image/jpeg, parsed=true, width=640, height=480, pixel-aspect-ratio=1/1, framerate=30/1; image/jpeg, parsed=true, width=640, height=360, pixel-aspect-ratio=1/1, framerate=30/1";

/// avfvideosrc's `fixate` (avfvideosrc.m 1.28.6): truncate to the first
/// structure, height to the maximum, framerate nearest 30, then fixate the
/// rest.
fn avf_fixate(caps: gst::Caps) -> gst::Caps {
    let mut caps = caps;
    caps.truncate();
    {
        let s = caps.make_mut().structure_mut(0).unwrap();
        s.fixate_field_nearest_int("height", i32::MAX);
        s.fixate_field_nearest_fraction("framerate", gst::Fraction::new(30, 1));
    }
    caps.fixate();
    caps
}

#[test]
fn constraints_land_the_dell_on_a_mode_it_supports() {
    if gst::init().is_err() {
        eprintln!("skip: GStreamer unavailable");
        return;
    }
    let device: gst::Caps = DELL_WB7022.parse().unwrap();
    let constraints = super::source_constraints();
    // Either intersection order (source first, or filter first) must give
    // a valid mode.
    for allowed in [
        device.intersect_with_mode(&constraints, gst::CapsIntersectMode::First),
        constraints.intersect_with_mode(&device, gst::CapsIntersectMode::First),
    ] {
        assert!(!allowed.is_empty());
        let fixed = avf_fixate(allowed);
        let s = fixed.structure(0).unwrap();
        assert!(!fixed.features(0).unwrap().is_any());
        assert_eq!(s.get::<i32>("width").unwrap(), 1280);
        assert_eq!(s.get::<i32>("height").unwrap(), 720);
        assert_eq!(
            s.get::<gst::Fraction>("framerate").unwrap(),
            gst::Fraction::new(30, 1),
            "a rate the device lists for 720p"
        );
        assert!(
            device.can_intersect(&fixed),
            "the fixed caps are one of the device's own modes: {fixed}"
        );
    }
}

#[test]
fn constraints_leave_the_linux_webcam_modes_unchanged() {
    if gst::init().is_err() {
        eprintln!("skip: GStreamer unavailable");
        return;
    }
    let device: gst::Caps = INTEGRATED_WEBCAM_HD.parse().unwrap();
    let allowed =
        device.intersect_with_mode(&super::source_constraints(), gst::CapsIntersectMode::First);
    assert_eq!(
        allowed.size(),
        device.size(),
        "a 720p camera keeps every mode: v4l2src chooses as before"
    );
}
