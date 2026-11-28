use super::{CallEvent, CallOutcome, CallParticipant, DirectCallInfo, FocusReason};

#[test]
fn a_group_call_connects_without_a_peer() {
    let group = CallEvent::Connected {
        call_id: "c1".into(),
        group_id: None,
        direct: None,
    };
    let one_to_one = CallEvent::Connected {
        call_id: "c2".into(),
        group_id: None,
        direct: Some(DirectCallInfo {
            kind: "video".into(),
            peer_key: "pk".into(),
            peer_display_name: "Ada".into(),
            expected_local_camera: true,
        }),
    };
    assert_eq!(group.call_id(), Some("c1"));
    assert_eq!(one_to_one.call_id(), Some("c2"));
    // The four direct-call facts travel together or not at all —
    // there is no state where the peer key is known but the kind
    // is not.
    assert!(matches!(group, CallEvent::Connected { direct: None, .. }));
}

#[test]
fn every_variant_reports_its_call_id() {
    for event in sample_events() {
        if matches!(event, CallEvent::SelfMuteChanged { .. }) {
            assert_eq!(event.call_id(), None, "our own mute names no call");
        } else {
            assert!(
                event.call_id().is_some_and(|id| !id.is_empty()),
                "every call event names its call"
            );
        }
    }
}

/// The daemon IPC is postcard: `flatten` and `tag = "..."` fail
/// there at runtime, so only a real round trip catches them.
#[test]
fn postcard_round_trips_every_variant() {
    for event in sample_events() {
        let bytes = postcard::to_allocvec(&event).expect("postcard encode");
        let back: CallEvent = postcard::from_bytes(&bytes).expect("postcard decode");
        assert_eq!(format!("{event:?}"), format!("{back:?}"));
    }
}

#[test]
fn host_serves_the_call_outcome_notices() {
    assert_eq!(CallOutcome::Declined.notice(), "Call declined");
    assert_eq!(CallOutcome::TimedOut.notice(), "Call timed out — no answer");
    assert_eq!(
        CallOutcome::Unreachable.notice(),
        "Could not reach the other caller"
    );
    assert_eq!(CallOutcome::Ended.notice(), "Call ended");
}

#[test]
fn focus_reasons_serialize_as_kebab_case_codes() {
    for (reason, expected) in [
        (FocusReason::CallStarted, "call-started"),
        (FocusReason::CallAccepted, "call-accepted"),
    ] {
        let event = CallEvent::FocusConversation {
            call_id: "c1".into(),
            peer_key: "pk".into(),
            peer_display_name: "Ada".into(),
            reason,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["focusConversation"]["reason"], expected);
        let bytes = postcard::to_allocvec(&event).unwrap();
        let round_trip = postcard::from_bytes::<CallEvent>(&bytes).unwrap();
        assert!(
            matches!(round_trip, CallEvent::FocusConversation { reason: actual, .. } if actual == reason)
        );
    }
}

/// Pins the JSON `src/ipc/channels/chat_events.ts` parses.
#[test]
fn json_shape_is_what_the_webview_parses() {
    let event = CallEvent::Ended {
        call_id: "c1".into(),
        notice: CallOutcome::Ended.notice(),
        peer_reason: None,
    };
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"ended":{"callId":"c1","notice":"Call ended","peerReason":null}}"#
    );

    let connected = CallEvent::Connected {
        call_id: "c1".into(),
        group_id: None,
        direct: Some(DirectCallInfo {
            kind: "audio".into(),
            peer_key: "pk".into(),
            peer_display_name: "Ada".into(),
            expected_local_camera: false,
        }),
    };
    assert_eq!(
        serde_json::to_string(&connected).unwrap(),
        r#"{"connected":{"callId":"c1","direct":{"kind":"audio","peerKey":"pk","peerDisplayName":"Ada","expectedLocalCamera":false},"groupId":null}}"#
    );
}

fn sample_events() -> Vec<CallEvent> {
    vec![
        CallEvent::Incoming {
            call_id: "c".into(),
            from: "pk".into(),
            display_name: "Ada".into(),
            kind: "video".into(),
            participants: ["a", "b"]
                .into_iter()
                .map(|key| CallParticipant {
                    public_key: key.into(),
                    display_name: format!("Name of {key}"),
                    is_self: key == "me",
                })
                .collect(),
            is_group: true,
            expires_at_ms: 1,
            group_id: None,
        },
        CallEvent::Ringing {
            call_id: "c".into(),
        },
        CallEvent::Started {
            call_id: "c".into(),
            kind: "audio".into(),
            peer_key: "pk".into(),
            peer_display_name: "Ada".into(),
            expires_at_ms: 1,
        },
        CallEvent::GroupStarted {
            call_id: "c".into(),
            kind: "audio".into(),
            participants: ["me", "b"]
                .into_iter()
                .map(|key| CallParticipant {
                    public_key: key.into(),
                    display_name: format!("Name of {key}"),
                    is_self: key == "me",
                })
                .collect(),
            group_id: Some("g".into()),
            expires_at_ms: 1,
        },
        CallEvent::Connected {
            call_id: "c".into(),
            group_id: None,
            direct: None,
        },
        CallEvent::Declined {
            call_id: "c".into(),
            notice: CallOutcome::Declined.notice(),
            peer_reason: Some("busy".into()),
        },
        CallEvent::Missed {
            call_id: "c".into(),
            from: "pk".into(),
        },
        CallEvent::TimedOut {
            call_id: "c".into(),
            notice: CallOutcome::TimedOut.notice(),
        },
        CallEvent::Unreachable {
            call_id: "c".into(),
            notice: CallOutcome::Unreachable.notice(),
        },
        CallEvent::FocusConversation {
            call_id: "c".into(),
            peer_key: "pk".into(),
            peer_display_name: "Ada".into(),
            reason: FocusReason::CallStarted,
        },
        CallEvent::Ended {
            call_id: "c".into(),
            notice: CallOutcome::Ended.notice(),
            peer_reason: None,
        },
        CallEvent::MediaStateChanged {
            call_id: "c".into(),
            audio: true,
            video: false,
            screen: false,
            timestamp_ms: 1,
        },
        CallEvent::ReactionReceived {
            call_id: "c".into(),
            sender: "pk".into(),
            emoji: "👍".into(),
            timestamp_ms: 1,
        },
        CallEvent::ParticipantJoined {
            call_id: "c".into(),
            participant_pubkey: "pk".into(),
        },
        CallEvent::ParticipantLeft {
            call_id: "c".into(),
            participant_pubkey: "pk".into(),
            reason: "left".into(),
        },
        CallEvent::SelfMuteChanged {
            muted: true,
            timestamp_ms: 1,
        },
    ]
}
