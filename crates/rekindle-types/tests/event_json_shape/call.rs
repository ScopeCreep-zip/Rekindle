//! `CallEvent` samples. `Connected` appears twice: with a
//! `DirectCallInfo` (1:1) and without (group), the two shapes the
//! webview distinguishes.

use rekindle_types::subscription_events::{CallEvent, DirectCallInfo, SubscriptionEvent};

use crate::{id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "incoming",
    "ringing",
    "started",
    "connected",
    "declined",
    "missed",
    "timedOut",
    "ended",
    "mediaStateChanged",
    "reactionReceived",
    "participantJoined",
    "participantLeft",
];

pub fn variant_name(e: &CallEvent) -> &'static str {
    match e {
        CallEvent::Incoming { .. } => "incoming",
        CallEvent::Ringing { .. } => "ringing",
        CallEvent::Started { .. } => "started",
        CallEvent::Connected { .. } => "connected",
        CallEvent::Declined { .. } => "declined",
        CallEvent::Missed { .. } => "missed",
        CallEvent::TimedOut { .. } => "timedOut",
        CallEvent::Ended { .. } => "ended",
        CallEvent::MediaStateChanged { .. } => "mediaStateChanged",
        CallEvent::ReactionReceived { .. } => "reactionReceived",
        CallEvent::ParticipantJoined { .. } => "participantJoined",
        CallEvent::ParticipantLeft { .. } => "participantLeft",
    }
}

fn ev(name: &str, e: CallEvent) -> Sample {
    (format!("call.{name}"), SubscriptionEvent::Call(e))
}

fn call_id() -> String {
    id("call_", '9')
}

pub fn samples() -> Vec<Sample> {
    let mut all = setup();
    all.extend(in_call());
    all
}

fn setup() -> Vec<Sample> {
    vec![
        ev(
            "incoming",
            CallEvent::Incoming {
                call_id: call_id(),
                from: pk('b'),
                display_name: text("Bob"),
                kind: text("video"),
                participants: vec![pk('b'), pk('c')],
                is_group: true,
                expires_at_ms: 1_760_000_030_000,
            },
        ),
        ev("ringing", CallEvent::Ringing { call_id: call_id() }),
        ev(
            "started",
            CallEvent::Started {
                call_id: call_id(),
                kind: text("audio"),
                peer_key: pk('b'),
                peer_display_name: text("Bob"),
                expires_at_ms: 1_760_000_030_000,
            },
        ),
        ev(
            "connected.direct",
            CallEvent::Connected {
                call_id: call_id(),
                direct: Some(DirectCallInfo {
                    kind: text("video"),
                    peer_key: pk('b'),
                    peer_display_name: text("Bob"),
                    expected_local_camera: true,
                }),
            },
        ),
        ev(
            "connected.group",
            CallEvent::Connected {
                call_id: call_id(),
                direct: None,
            },
        ),
        ev(
            "declined",
            CallEvent::Declined {
                call_id: call_id(),
                reason: text("busy"),
            },
        ),
        ev(
            "missed",
            CallEvent::Missed {
                call_id: call_id(),
                from: pk('b'),
            },
        ),
        ev("timedOut", CallEvent::TimedOut { call_id: call_id() }),
    ]
}

fn in_call() -> Vec<Sample> {
    vec![
        ev(
            "ended",
            CallEvent::Ended {
                call_id: call_id(),
                reason: text("hangup"),
            },
        ),
        ev(
            "mediaStateChanged",
            CallEvent::MediaStateChanged {
                call_id: call_id(),
                audio: true,
                video: true,
                screen: false,
                timestamp_ms: 1_760_000_040_000,
            },
        ),
        ev(
            "reactionReceived",
            CallEvent::ReactionReceived {
                call_id: call_id(),
                sender: pk('b'),
                emoji: text("👍"),
                timestamp_ms: 1_760_000_041_000,
            },
        ),
        ev(
            "participantJoined",
            CallEvent::ParticipantJoined {
                call_id: call_id(),
                participant_pubkey: pk('c'),
            },
        ),
        ev(
            "participantLeft",
            CallEvent::ParticipantLeft {
                call_id: call_id(),
                participant_pubkey: pk('c'),
                reason: text("left"),
            },
        ),
    ]
}
