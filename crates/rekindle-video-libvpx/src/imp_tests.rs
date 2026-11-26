use super::*;
use rekindle_types::video::ScalabilityMode;

fn constraints(codec: Codec, w: u32, h: u32) -> EncoderConstraints {
    EncoderConstraints {
        codec,
        max_width: w,
        max_height: h,
        max_fps: 15,
        scalability_mode: ScalabilityMode::Flat,
    }
}

/// A synthetic I420 frame with a gradient luma plane (real content, so
/// rate control has something to compress).
fn synthetic_frame(w: u32, h: u32, ts: u64) -> RawFrame {
    let mut y = vec![0u8; RawFrame::expected_y_len(w, h)];
    for (i, px) in y.iter_mut().enumerate() {
        *px = u8::try_from(i % 256).unwrap_or(0);
    }
    RawFrame {
        width: w,
        height: h,
        y,
        u: vec![0x60; RawFrame::expected_chroma_len(w, h)],
        v: vec![0xA0; RawFrame::expected_chroma_len(w, h)],
        timestamp_ms: ts,
    }
}

#[test]
fn encodes_synthetic_vp9_keyframe() {
    let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
    enc.configure(&constraints(Codec::Vp9, 320, 240)).unwrap();
    let out = enc
        .encode(synthetic_frame(320, 240, 0))
        .unwrap()
        .expect("realtime VP9 must emit the first frame (lag=0)");
    assert!(
        !out.payload.is_empty(),
        "encoded VP9 payload must be non-empty"
    );
    assert!(out.keyframe, "first frame must be a keyframe");
    assert_eq!(out.timestamp_ms, 0);
    assert_eq!(enc.codec(), Codec::Vp9);
    // Observed on macOS + libvpx 1.15.2: ~3.5 KB for this 320×240 frame.
    assert!(
        out.payload.len() > 100,
        "keyframe unexpectedly tiny: {} bytes",
        out.payload.len()
    );
}

#[test]
fn encodes_synthetic_vp8_keyframe() {
    let mut enc = LibvpxEncoder::new(Codec::Vp8).unwrap();
    enc.configure(&constraints(Codec::Vp8, 320, 240)).unwrap();
    let out = enc
        .encode(synthetic_frame(320, 240, 0))
        .unwrap()
        .expect("realtime VP8 must emit the first frame");
    assert!(
        !out.payload.is_empty(),
        "encoded VP8 payload must be non-empty"
    );
    assert!(out.keyframe, "first frame must be a keyframe");
    assert_eq!(enc.codec(), Codec::Vp8);
    // Observed on macOS + libvpx 1.15.2: ~1.5 KB for this 320×240 frame.
    assert!(
        out.payload.len() > 100,
        "keyframe tiny: {} bytes",
        out.payload.len()
    );
}

#[test]
fn force_keyframe_makes_next_frame_a_keyframe() {
    let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
    enc.configure(&constraints(Codec::Vp9, 160, 120)).unwrap();
    // First frame is a keyframe; feed a couple inter frames.
    let _ = enc.encode(synthetic_frame(160, 120, 0)).unwrap();
    let _ = enc.encode(synthetic_frame(160, 120, 66)).unwrap();
    enc.force_keyframe();
    let kf = enc
        .encode(synthetic_frame(160, 120, 132))
        .unwrap()
        .expect("post-force frame produces output");
    assert!(kf.keyframe, "frame after force_keyframe must be a keyframe");
}

#[test]
fn set_bitrate_live_does_not_error() {
    let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
    enc.configure(&constraints(Codec::Vp9, 160, 120)).unwrap();
    let first = enc.encode(synthetic_frame(160, 120, 0)).unwrap();
    assert!(first.is_some());
    // Live retarget (no keyframe, no rebuild) then keep encoding.
    enc.set_bitrate(120);
    let next = enc.encode(synthetic_frame(160, 120, 66)).unwrap();
    // May be None if the RC dropped it, but must not error and must not
    // force a keyframe.
    if let Some(f) = next {
        assert!(!f.keyframe, "a bitrate change must not force a keyframe");
    }
}

#[test]
fn encode_before_configure_errors() {
    let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
    assert!(matches!(
        enc.encode(synthetic_frame(64, 64, 0)),
        Err(VideoError::EncoderNotConfigured)
    ));
}

#[test]
fn h264_is_unsupported() {
    // libvpx has no H.264 encoder — rejected at construction and at
    // configure.
    assert!(matches!(
        LibvpxEncoder::new(Codec::H264),
        Err(VideoError::Unsupported(_))
    ));
    let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
    assert!(matches!(
        enc.configure(&constraints(Codec::H264, 320, 240)),
        Err(VideoError::Unsupported(_))
    ));
}

#[test]
fn odd_dimensions_rejected_cleanly() {
    let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
    enc.configure(&constraints(Codec::Vp9, 320, 240)).unwrap();
    let mut f = synthetic_frame(320, 240, 0);
    f.width = 321;
    f.y = vec![0; 321 * 240];
    f.u = vec![0; RawFrame::expected_chroma_len(321, 240)];
    f.v = vec![0; RawFrame::expected_chroma_len(321, 240)];
    assert!(matches!(enc.encode(f), Err(VideoError::InvalidInput(_))));
}
