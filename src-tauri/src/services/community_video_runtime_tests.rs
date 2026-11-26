use super::*;
use std::sync::Arc;

/// Phase 1 — the media-ready gate is the FIRST check: a default
/// AppState (no voice session, no handshake) must reject the frame
/// with the gate's reason and count the drop.
#[test]
fn send_video_frame_rejected_before_media_ready() {
    let state: SharedState = Arc::new(crate::state::AppState::default());
    let request = SendVideoFrameRequest {
        stream_id_hex: "00".repeat(16),
        frame_seq: 1,
        keyframe: true,
        codec: "vp9".to_string(),
        timestamp: 0,
        encoded_payload_b64: "AAAA".to_string(),
    };
    let err = send_video_frame_inner(&state, "c1", "ch1", &request)
        .expect_err("default state must be gated");
    assert!(
        err.starts_with("media not ready:"),
        "gate must name itself: got {err}"
    );
    assert!(
        err.contains("not-in-voice"),
        "reason names the blocker: {err}"
    );
    assert_eq!(
        state
            .video_pre_ready_drops
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "drop counter increments"
    );
}
