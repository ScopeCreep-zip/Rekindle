use super::*;

fn community_scope() -> VoiceScope {
    VoiceScope::Community {
        community: "c".into(),
        channel: "ch".into(),
    }
}

#[test]
fn a_dm_call_is_expressible_and_has_no_community() {
    let dm = VoiceScope::Dm {
        peer_key: "peer".into(),
    };
    assert_eq!(dm.community(), None);
    assert_eq!(dm.call_type(), "dm");
    assert_eq!(dm.session_key(), "peer");

    let event = VoiceEvent::LocalJoined { scope: dm };
    assert_eq!(event.community(), None, "a DM call belongs to no community");
}

#[test]
fn device_changes_are_machine_wide() {
    let event = VoiceEvent::DeviceChanged {
        device_type: "input".into(),
        device_name: "Yeti".into(),
        reason: "unplugged".into(),
    };
    assert!(
        event.scope().is_none(),
        "a device can change with no call running"
    );
}

#[test]
fn community_scope_reports_its_community() {
    let event = VoiceEvent::RosterUpdated {
        scope: community_scope(),
        participants: vec![VoiceParticipant {
            pseudonym_key: "p".into(),
            display_name: "Ada".into(),
        }],
    };
    assert_eq!(event.community(), Some("c"));
    assert_eq!(event.scope().unwrap().session_key(), "ch");
}

/// The daemon IPC is postcard: `flatten` and `tag = "..."` fail
/// there at runtime, so only a real round trip catches them.
#[test]
fn postcard_round_trips_every_variant() {
    let events = vec![
        VoiceEvent::Joined {
            scope: community_scope(),
            pseudonym: "p".into(),
            display_name: "Ada".into(),
            route_blob: Some(vec![1, 2, 3]),
        },
        VoiceEvent::Left {
            scope: community_scope(),
            pseudonym: "p".into(),
        },
        VoiceEvent::ModeChanged {
            scope: community_scope(),
            mode: "stage".into(),
            host_pseudonym: None,
        },
        VoiceEvent::MuteChanged {
            scope: community_scope(),
            target_pseudonym: "p".into(),
            muted: true,
        },
        VoiceEvent::DeafenChanged {
            scope: community_scope(),
            target_pseudonym: "p".into(),
            deafened: false,
        },
        VoiceEvent::RosterUpdated {
            scope: community_scope(),
            participants: vec![VoiceParticipant {
                pseudonym_key: "p".into(),
                display_name: "abab abab".into(),
            }],
        },
        VoiceEvent::JoinHandshake {
            scope: community_scope(),
            state: "offered".into(),
            peer: Some("p".into()),
            display_name: None,
        },
        VoiceEvent::PeerConfirmed {
            scope: community_scope(),
            pseudonym: "p".into(),
        },
        VoiceEvent::MediaReady {
            scope: community_scope(),
            ready: true,
            reason: "roster_non_empty".into(),
        },
        VoiceEvent::StageUpdated {
            scope: community_scope(),
            topic: Some("AMA".into()),
            speakers: vec!["p".into()],
            moderator_pseudonym: "m".into(),
        },
        VoiceEvent::SpeakRequested {
            scope: community_scope(),
            requester_pseudonym: "p".into(),
        },
        VoiceEvent::SpeakResponded {
            scope: community_scope(),
            requester_pseudonym: "p".into(),
            granted: true,
            moderator_pseudonym: "m".into(),
        },
        VoiceEvent::LocalJoined {
            scope: VoiceScope::Dm {
                peer_key: "peer".into(),
            },
        },
        VoiceEvent::SpeakingChanged {
            scope: community_scope(),
            pseudonym: "p".into(),
            speaking: true,
        },
        VoiceEvent::DeviceChanged {
            device_type: "input".into(),
            device_name: "Yeti".into(),
            reason: "user".into(),
        },
        VoiceEvent::PacketsDropped {
            scope: community_scope(),
            reason: "voice loop".into(),
            count: 12,
        },
        VoiceEvent::ConnectionQuality {
            scope: community_scope(),
            quality: "good".into(),
            rx_overflow_drops: 1,
            rx_late_drops: 2,
            rx_key_drops: 3,
            ingress_drops: 4,
            // Populated: pins `Option<u32>` and `f32` through
            // postcard, the two shapes in `LinkMeasurement` that a
            // non-self-describing format could get wrong.
            link: Some(LinkMeasurement {
                loss_q8: 13,
                discard_q8: 4,
                jitter_ms: 27,
                rtt_ms: Some(85),
                r_factor: 78,
                mos_lq: 4.1,
                mos_cq: 3.9,
                bitrate_bps: 32_000,
                one_way_p50_ms: Some(120),
                one_way_p95_ms: Some(180),
                mouth_to_ear_ms: None,
                outside_g114: false,
            }),
        },
        // And the pre-first-report case: no peer has reported, so
        // `quality` came from local send-failure counts.
        VoiceEvent::ConnectionQuality {
            scope: community_scope(),
            quality: "recovering".into(),
            rx_overflow_drops: 0,
            rx_late_drops: 0,
            rx_key_drops: 0,
            ingress_drops: 0,
            link: None,
        },
    ];
    for event in events {
        let bytes = postcard::to_allocvec(&event).expect("postcard encode");
        let back: VoiceEvent = postcard::from_bytes(&bytes).expect("postcard decode");
        assert_eq!(format!("{event:?}"), format!("{back:?}"));
    }
}

/// Pins the JSON `src/ipc/channels/voice_events.ts` parses.
#[test]
fn json_shape_is_what_the_webview_parses() {
    let event = VoiceEvent::LocalJoined {
        scope: community_scope(),
    };
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"localJoined":{"scope":{"community":{"community":"c","channel":"ch"}}}}"#
    );

    let dm = VoiceEvent::Joined {
        scope: VoiceScope::Dm {
            peer_key: "peer".into(),
        },
        pseudonym: "p".into(),
        display_name: "Ada".into(),
        route_blob: None,
    };
    assert_eq!(
        serde_json::to_string(&dm).unwrap(),
        r#"{"joined":{"scope":{"dm":{"peerKey":"peer"}},"pseudonym":"p","displayName":"Ada","routeBlob":null}}"#
    );
}
