//! Wire-format snapshot tests for the video cluster of `CommunityEvent`.
//!
//! These assert the EXACT JSON each variant serializes to. They were
//! written against the original struct-variant shapes BEFORE the
//! newtype-payload conversion, and pass unchanged after it — the proof
//! that `X { fields… }` → `X(XEvent { fields… })` under
//! `serde(tag = "type", content = "data")` is wire-identical, and that
//! the frontend TS contract (`src/ipc/channels.ts`) is untouched.

use rekindle_types::video::{Codec, ScalabilityMode};
use rekindle_video::{DecoderConstraints, EncoderConstraints, SessionVideoConfig};
use serde_json::json;

use super::*;

#[test]
fn video_keyframe_request_wire() {
    let ev = CommunityEvent::VideoKeyframeRequest(VideoKeyframeRequestEvent {
        community_id: "c1".into(),
        sender_pseudonym: "p1".into(),
        channel_id: "ch1".into(),
        stream_id: "s1".into(),
    });
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "videoKeyframeRequest",
            "data": {
                "communityId": "c1",
                "senderPseudonym": "p1",
                "channelId": "ch1",
                "streamId": "s1"
            }
        })
    );
}

#[test]
fn video_session_config_wire() {
    let config = SessionVideoConfig {
        encoder: EncoderConstraints {
            codec: Codec::Vp9,
            max_width: 854,
            max_height: 480,
            max_fps: 15,
            scalability_mode: ScalabilityMode::Flat,
        },
        decoder: DecoderConstraints {
            optimize_for_latency: false,
        },
    };
    let ev = CommunityEvent::VideoSessionConfig(VideoSessionConfigEvent {
        community_id: "c1".into(),
        channel_id: "ch1".into(),
        config: config.clone(),
    });
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "videoSessionConfig",
            "data": {
                "communityId": "c1",
                "channelId": "ch1",
                "config": serde_json::to_value(&config).unwrap()
            }
        })
    );
}

#[test]
fn video_codec_incompatible_wire() {
    let ev = CommunityEvent::VideoCodecIncompatible(VideoCodecIncompatibleEvent {
        community_id: "c1".into(),
        channel_id: "ch1".into(),
        peers: vec!["p1".into(), "p2".into()],
    });
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "videoCodecIncompatible",
            "data": {
                "communityId": "c1",
                "channelId": "ch1",
                "peers": ["p1", "p2"]
            }
        })
    );
}

#[test]
fn video_bitrate_target_wire() {
    let ev = CommunityEvent::VideoBitrateTarget(VideoBitrateTargetEvent {
        community_id: "c1".into(),
        channel_id: "ch1".into(),
        kbps: 950,
    });
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "videoBitrateTarget",
            "data": { "communityId": "c1", "channelId": "ch1", "kbps": 950 }
        })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn native_video_error_wire() {
    let ev = CommunityEvent::NativeVideoError(NativeVideoErrorEvent {
        community_id: "c1".into(),
        channel_id: "ch1".into(),
        message: "camera unplugged".into(),
    });
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "nativeVideoError",
            "data": {
                "communityId": "c1",
                "channelId": "ch1",
                "message": "camera unplugged"
            }
        })
    );
}

#[test]
fn video_envelope_rejected_wire() {
    let ev = CommunityEvent::VideoEnvelopeRejected(VideoEnvelopeRejectedEvent {
        community_id: "c1".into(),
        sender_pseudonym: "<unknown>".into(),
        reason: "bad signature".into(),
    });
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "videoEnvelopeRejected",
            "data": {
                "communityId": "c1",
                "senderPseudonym": "<unknown>",
                "reason": "bad signature"
            }
        })
    );
}

#[test]
fn video_topology_change_wire() {
    let ev = CommunityEvent::VideoTopologyChange(VideoTopologyChangeEvent {
        community_id: "c1".into(),
        sender_pseudonym: "p1".into(),
        channel_id: "ch1".into(),
        stream_id: "s1".into(),
        relay_host_pseudonym: None,
        reason: "initial".into(),
        lamport: 7,
    });
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "videoTopologyChange",
            "data": {
                "communityId": "c1",
                "senderPseudonym": "p1",
                "channelId": "ch1",
                "streamId": "s1",
                "relayHostPseudonym": null,
                "reason": "initial",
                "lamport": 7
            }
        })
    );
}
