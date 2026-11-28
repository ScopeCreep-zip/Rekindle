//! Wire-format snapshot tests for the video cluster of `CommunityEvent`.
use rekindle_governance_runtime::{deps::GovernanceRuntimeDeps, CardText};
use rekindle_types::id::PseudonymKey;
use rekindle_types::video::{Codec, ScalabilityMode};
use rekindle_video::{DecoderConstraints, EncoderConstraints, SessionVideoConfig};
use serde_json::json;

use super::*;
use crate::services::video_adapter::{incompatible_event, rejection_event};
use crate::test_support::TestHost;

const NAMED_PEER: PseudonymKey = PseudonymKey([0xab; 32]);
const UNNAMED_PEER: PseudonymKey = PseudonymKey([0xcd; 32]);

async fn host_with_card() -> TestHost {
    let test = TestHost::start().await;
    test.host.governance().note_member_card(
        "c1",
        &NAMED_PEER,
        CardText {
            display_name: Some("Kit".into()),
            custom_status: None,
            game: None,
        },
    );
    test
}

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

#[tokio::test]
async fn video_codec_incompatible_wire() {
    let test = host_with_card().await;
    let named = NAMED_PEER.to_hex();
    let unnamed = UNNAMED_PEER.to_hex();
    let ev = incompatible_event(
        &test.state,
        "c1",
        "ch1",
        vec![named.clone(), unnamed.clone()],
    )
    .expect("video incompatibility event");
    assert_eq!(
        serde_json::to_value(&ev).unwrap(),
        json!({
            "type": "videoCodecIncompatible",
            "data": {
                "communityId": "c1",
                "channelId": "ch1",
                "peers": [
                    {"pseudonymKey": named, "displayName": "Kit"},
                    {"pseudonymKey": unnamed, "displayName": "cdcd cdcd"}
                ]
            }
        })
    );
    test.shutdown().await;
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

#[tokio::test]
async fn video_envelope_rejected_wire() {
    let test = host_with_card().await;
    for (peer, name) in [(NAMED_PEER, "Kit"), (UNNAMED_PEER, "cdcd cdcd")] {
        let pseudonym = peer.to_hex();
        let ev = rejection_event(
            &test.state,
            "c1".into(),
            pseudonym.clone(),
            "bad signature".into(),
        )
        .expect("video rejection event");
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({
                "type": "videoEnvelopeRejected",
                "data": {
                    "communityId": "c1",
                    "senderPseudonym": pseudonym,
                    "displayName": name,
                    "reason": "bad signature"
                }
            })
        );
    }
    test.shutdown().await;
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
