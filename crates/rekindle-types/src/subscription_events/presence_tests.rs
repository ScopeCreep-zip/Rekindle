use super::*;

#[test]
fn idle_is_distinguishable_from_unobserved() {
    let unobserved = PresenceSnapshot::status("online");
    let stopped = PresenceSnapshot::status("online").with_game(GameActivity::Idle);

    assert!(unobserved.game.is_none(), "no game observation was made");
    assert_eq!(stopped.game, Some(GameActivity::Idle), "observed, stopped");
    // Both report no game name — the difference is whether anyone looked.
    assert_eq!(unobserved.game_name(), None);
    assert_eq!(stopped.game_name(), None);
}

#[test]
fn from_parts_round_trips_a_running_game() {
    let g = GameActivity::from_parts(
        Some("Battlefield 2".into()),
        Some(7),
        Some(1234),
        Some("10.0.0.1:16567".into()),
        Some("xfire/4101".into()),
    );
    assert_eq!(g.game_name(), Some("Battlefield 2"));
    assert_eq!(g.game_id(), Some(7));
    assert_eq!(
        GameActivity::from_parts(None, Some(7), None, None, None),
        GameActivity::Idle
    );
}

/// The daemon IPC is postcard, which cannot decode `flatten` or a
/// `tag = "..."` enum. Those fail at *runtime*, so only a real
/// round trip catches them — a compile pass proves nothing here.
/// Pins the exact JSON `src/ipc/channels/presence_events.ts` parses.
/// The webview is the only JSON consumer of this enum, so a silent
/// rename here is a silent frontend break — the TS types are written
/// against these literals.
#[test]
fn json_shape_is_what_the_webview_parses() {
    let playing = PresenceEvent::SelfChanged {
        public_key: "abc".into(),
        snapshot: PresenceSnapshot::game(GameActivity::Playing {
            game_name: "BF2".into(),
            game_id: Some(7),
            elapsed_seconds: Some(12),
            server_address: None,
            icon_key: Some("xfire/8507".into()),
            joinable: false,
            joinable_reason: Some(crate::friend::NotJoinableReason::NoServer),
        }),
    };
    assert_eq!(
        serde_json::to_string(&playing).unwrap(),
        r#"{"selfChanged":{"publicKey":"abc","snapshot":{"status":null,"statusMessage":null,"game":{"playing":{"gameName":"BF2","gameId":7,"elapsedSeconds":12,"serverAddress":null,"iconKey":"xfire/8507","joinable":false,"joinableReason":"noServer"}},"idle":null}}}"#
    );

    let member = PresenceEvent::CommunityMemberChanged {
        community: "c".into(),
        pseudonym: "p".into(),
        snapshot: PresenceSnapshot::status("away"),
    };
    assert_eq!(
        serde_json::to_string(&member).unwrap(),
        r#"{"communityMemberChanged":{"community":"c","pseudonym":"p","snapshot":{"status":"away","statusMessage":null,"game":null,"idle":null}}}"#
    );

    // A unit variant is a bare string, not an object — the TS union
    // has to accept `"idle"` alongside `{ playing: {...} }`.
    assert_eq!(
        serde_json::to_string(&PresenceSnapshot::game(GameActivity::Idle)).unwrap(),
        r#"{"status":null,"statusMessage":null,"game":"idle","idle":null}"#
    );
}

#[test]
fn postcard_round_trips_every_variant() {
    let full = PresenceSnapshot::status("online")
        .with_status_message(Some("brb".into()))
        .with_game(GameActivity::Playing {
            game_name: "Battlefield 2".into(),
            game_id: Some(7),
            elapsed_seconds: Some(1234),
            server_address: Some("10.0.0.1:16567".into()),
            icon_key: Some("xfire/4101".into()),
            joinable: false,
            joinable_reason: Some(crate::friend::NotJoinableReason::CannotLaunch),
        });

    for event in [
        PresenceEvent::CommunityMemberChanged {
            community: "c".into(),
            pseudonym: "p".into(),
            snapshot: full.clone(),
        },
        PresenceEvent::FriendChanged {
            peer_key: "k".into(),
            snapshot: PresenceSnapshot::game(GameActivity::Idle),
        },
        PresenceEvent::SelfChanged {
            public_key: "me".into(),
            snapshot: PresenceSnapshot::status("away")
                .with_idle(crate::presence::IdleState::AutoAway),
        },
        PresenceEvent::SelfProfileChanged {
            public_key: "me".into(),
            display_name: Some("Kali".into()),
            avatar_changed: true,
        },
    ] {
        let bytes = postcard::to_allocvec(&event).expect("postcard encode");
        let back: PresenceEvent = postcard::from_bytes(&bytes).expect("postcard decode");
        assert_eq!(
            format!("{event:?}"),
            format!("{back:?}"),
            "postcard round trip must preserve the event exactly"
        );
    }
}

#[test]
fn every_variant_exposes_its_snapshot() {
    let snap = PresenceSnapshot::status("away").with_status_message(Some("brb".into()));
    for event in [
        PresenceEvent::CommunityMemberChanged {
            community: "c".into(),
            pseudonym: "p".into(),
            snapshot: snap.clone(),
        },
        PresenceEvent::FriendChanged {
            peer_key: "k".into(),
            snapshot: snap.clone(),
        },
        PresenceEvent::SelfChanged {
            public_key: "me".into(),
            snapshot: snap.clone(),
        },
    ] {
        let snapshot = event.snapshot().expect("a presence observation");
        assert_eq!(snapshot.status.as_deref(), Some("away"));
        assert_eq!(snapshot.status_message.as_deref(), Some("brb"));
    }
}
