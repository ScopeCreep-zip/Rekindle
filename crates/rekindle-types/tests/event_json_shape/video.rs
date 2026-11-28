//! `VideoEvent` samples (plan E4.1 S2), each `MediaScope` covered.

use rekindle_types::media::MediaScope;
use rekindle_types::subscription_events::{SubscriptionEvent, VideoCodecPeer, VideoEvent};
use rekindle_types::video::{
    Codec, DecoderConstraints, EncoderConstraints, ScalabilityMode, SessionVideoConfig,
};

use crate::{pk, rk, Sample};

pub const NAMES: &[&str] = &[
    "keyframeRequest",
    "sessionConfig",
    "codecIncompatible",
    "bitrateTarget",
    "envelopeRejected",
    "topologyChange",
];

pub const NESTED: &[&str] = &["MediaScope.directCall", "MediaScope.channel"];

pub fn variant_name(e: &VideoEvent) -> &'static str {
    match e {
        VideoEvent::KeyframeRequest { .. } => "keyframeRequest",
        VideoEvent::SessionConfig { .. } => "sessionConfig",
        VideoEvent::CodecIncompatible { .. } => "codecIncompatible",
        VideoEvent::BitrateTarget { .. } => "bitrateTarget",
        VideoEvent::EnvelopeRejected { .. } => "envelopeRejected",
        VideoEvent::TopologyChange { .. } => "topologyChange",
    }
}

pub fn nested_names(e: &VideoEvent) -> Vec<&'static str> {
    vec![match e.scope() {
        MediaScope::DirectCall { .. } => "MediaScope.directCall",
        MediaScope::Channel { .. } => "MediaScope.channel",
    }]
}

fn channel() -> MediaScope {
    MediaScope::Channel {
        community_id: rk('c'),
        channel_id: "1".repeat(32),
    }
}

fn direct() -> MediaScope {
    MediaScope::DirectCall { peer: pk('b') }
}

fn ev(name: &str, e: VideoEvent) -> Sample {
    (format!("video.{name}"), SubscriptionEvent::Video(e))
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "keyframeRequest.channel",
            VideoEvent::KeyframeRequest {
                scope: channel(),
                sender: pk('d'),
                stream_id: "5".repeat(16),
            },
        ),
        ev(
            "keyframeRequest.directCall",
            VideoEvent::KeyframeRequest {
                scope: direct(),
                sender: pk('b'),
                stream_id: "6".repeat(16),
            },
        ),
        ev(
            "sessionConfig",
            VideoEvent::SessionConfig {
                scope: channel(),
                config: SessionVideoConfig {
                    encoder: EncoderConstraints {
                        codec: Codec::Vp9,
                        max_width: 1280,
                        max_height: 720,
                        max_fps: 30,
                        scalability_mode: ScalabilityMode::L1T2,
                    },
                    decoder: DecoderConstraints {
                        optimize_for_latency: true,
                    },
                },
            },
        ),
        ev(
            "codecIncompatible",
            VideoEvent::CodecIncompatible {
                scope: channel(),
                peers: vec![VideoCodecPeer {
                    sender: pk('e'),
                    display_name: "Eve".into(),
                }],
            },
        ),
        ev(
            "bitrateTarget",
            VideoEvent::BitrateTarget {
                scope: direct(),
                kbps: 1200,
            },
        ),
        ev(
            "envelopeRejected",
            VideoEvent::EnvelopeRejected {
                scope: channel(),
                sender: pk('f'),
                display_name: "Fern".into(),
                reason: "bad signature".into(),
            },
        ),
        ev(
            "topologyChange",
            VideoEvent::TopologyChange {
                scope: channel(),
                sender: pk('d'),
                stream_id: "5".repeat(16),
                relay_host: Some(pk('a')),
                reason: "relay elected".into(),
                lamport: 7,
            },
        ),
    ]
}

#[test]
fn scoped_video_events_round_trip_on_the_bus() {
    use rekindle_types::subscription_events::{EventCategory, EventScope};

    for (name, sample) in samples() {
        let SubscriptionEvent::Video(video) = &sample else {
            panic!("{name}: expected video");
        };
        let (scope, community) = match video.scope() {
            MediaScope::DirectCall { peer } => (EventScope::Peer(peer.clone()), None),
            MediaScope::Channel { community_id, .. } => (
                EventScope::Community(community_id.clone()),
                Some(community_id.as_str()),
            ),
        };
        assert_eq!(sample.category(), EventCategory::Video, "{name}");
        assert_eq!(sample.scope(), scope, "{name}");
        assert_eq!(sample.community(), community, "{name}");
        let bytes = postcard::to_allocvec(&sample).unwrap();
        let decoded = postcard::from_bytes::<SubscriptionEvent>(&bytes).unwrap();
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            serde_json::to_value(sample).unwrap(),
            "{name}"
        );
    }
}
