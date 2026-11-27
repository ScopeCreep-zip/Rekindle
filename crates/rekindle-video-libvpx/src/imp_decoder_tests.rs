use super::*;
use crate::LibvpxEncoder;
use rekindle_types::video::ScalabilityMode;
use rekindle_video::codec::VideoEncoder;
use rekindle_video::EncoderConstraints;

fn constraints(codec: Codec, w: u32, h: u32) -> EncoderConstraints {
    EncoderConstraints {
        codec,
        max_width: w,
        max_height: h,
        max_fps: 15,
        scalability_mode: ScalabilityMode::Flat,
    }
}

/// A synthetic I420 frame with a gradient luma plane and constant chroma
/// — real content, so the encoded frame is non-trivial and the decoded
/// luma must carry the gradient back.
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

/// Encode a synthetic frame then decode it back — proving the
/// encode↔decode pair end to end for one codec. Not pixel equality
/// (VP8/VP9 are lossy); we assert dimensions, plane sizes, that the
/// decoded luma is non-trivial, and that the wire timestamp survives.
fn round_trip(codec: Codec) -> (usize, RawFrame) {
    let (w, h) = (320u32, 240u32);

    let mut enc = LibvpxEncoder::new(codec).expect("encoder construction");
    enc.configure(&constraints(codec, w, h))
        .expect("encoder configure");
    let encoded = enc
        .encode(synthetic_frame(w, h, 7))
        .expect("encode must not error")
        .expect("realtime encoder emits the first frame (lag=0)");
    assert!(encoded.keyframe, "first encoded frame must be a keyframe");
    assert!(!encoded.payload.is_empty(), "encoded payload non-empty");

    let mut dec = LibvpxDecoder::new(codec).expect("decoder construction");
    assert_eq!(dec.codec(), codec);
    let decoded = dec
        .decode(&encoded)
        .expect("decode must not error")
        .expect("a keyframe must yield a displayable frame");

    // Right dimensions (from the bitstream, via d_w/d_h).
    assert_eq!(decoded.width, w, "decoded width");
    assert_eq!(decoded.height, h, "decoded height");
    // Plane sizes match tightly-packed I420 — proves stride was honored
    // (a naive stride*rows blit would over-copy the SIMD row padding).
    assert_eq!(
        decoded.y.len(),
        RawFrame::expected_y_len(w, h),
        "Y plane must be width*height, tightly packed"
    );
    assert_eq!(
        decoded.u.len(),
        RawFrame::expected_chroma_len(w, h),
        "U plane must be ceil(w/2)*ceil(h/2)"
    );
    assert_eq!(
        decoded.v.len(),
        RawFrame::expected_chroma_len(w, h),
        "V plane must be ceil(w/2)*ceil(h/2)"
    );
    // The reconstructed frame is itself a valid RawFrame.
    decoded.validate().expect("decoded frame validates");
    // The gradient luma survived (not a blank/zero plane).
    assert!(
        decoded.y.iter().any(|&b| b != 0),
        "decoded luma must carry the gradient"
    );
    // Wire timestamp passes straight through the decoder.
    assert_eq!(decoded.timestamp_ms, 7, "timestamp passthrough");

    (encoded.payload.len(), decoded)
}

#[test]
fn round_trip_vp9() {
    let (encoded_bytes, decoded) = round_trip(Codec::Vp9);
    // Observed on macOS + libvpx 1.15.2: 320×240 keyframe ~3–4 KB.
    assert!(
        encoded_bytes > 100,
        "VP9 keyframe unexpectedly tiny: {encoded_bytes} bytes"
    );
    assert_eq!(decoded.y.len(), 320 * 240);
    assert_eq!(decoded.u.len(), 160 * 120);
}

#[test]
fn round_trip_vp8() {
    let (encoded_bytes, decoded) = round_trip(Codec::Vp8);
    // Observed on macOS + libvpx 1.15.2: 320×240 keyframe ~1–2 KB.
    assert!(
        encoded_bytes > 100,
        "VP8 keyframe unexpectedly tiny: {encoded_bytes} bytes"
    );
    assert_eq!(decoded.y.len(), 320 * 240);
    assert_eq!(decoded.v.len(), 160 * 120);
}

#[test]
fn empty_payload_yields_none() {
    let mut dec = LibvpxDecoder::new(Codec::Vp9).unwrap();
    let empty = EncodedVideoFrame {
        payload: Vec::new(),
        keyframe: false,
        timestamp_ms: 0,
    };
    assert!(matches!(dec.decode(&empty), Ok(None)));
}

#[test]
fn h264_decoder_is_unsupported() {
    assert!(matches!(
        LibvpxDecoder::new(Codec::H264),
        Err(VideoError::Unsupported(_))
    ));
}
