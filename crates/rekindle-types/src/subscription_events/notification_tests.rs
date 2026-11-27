use super::*;

#[test]
fn postcard_round_trips_every_variant() {
    let events = vec![
        NotificationEvent::MessageReceived {
            title: "t".into(),
            body: "b".into(),
            community_id: "c".into(),
            channel_id: "ch".into(),
            sound_ref: Some("ping".into()),
        },
        NotificationEvent::SystemAlert {
            title: "t".into(),
            subject: None,
            body: "b".into(),
        },
        NotificationEvent::UpdateAvailable {
            version: "1.2.3".into(),
        },
        NotificationEvent::CallIncoming {
            call_id: "id".into(),
            from: "pk".into(),
            display_name: "Ada".into(),
            kind: "audio".into(),
            expires_at_ms: 1234,
            is_group: false,
        },
    ];
    for event in events {
        let bytes = postcard::to_allocvec(&event).expect("postcard encode");
        let back: NotificationEvent = postcard::from_bytes(&bytes).expect("postcard decode");
        assert_eq!(format!("{event:?}"), format!("{back:?}"));
    }
}

/// Pins the JSON `src/ipc/channels/notification_events.ts` parses.
#[test]
fn json_shape_is_what_the_webview_parses() {
    let event = NotificationEvent::UpdateAvailable {
        version: "1.2.3".into(),
    };
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"updateAvailable":{"version":"1.2.3"}}"#
    );

    let call = NotificationEvent::CallIncoming {
        call_id: "id".into(),
        from: "pk".into(),
        display_name: "Ada".into(),
        kind: "audio".into(),
        expires_at_ms: 1234,
        is_group: true,
    };
    assert_eq!(
        serde_json::to_string(&call).unwrap(),
        r#"{"callIncoming":{"callId":"id","from":"pk","displayName":"Ada","kind":"audio","expiresAtMs":1234,"isGroup":true}}"#
    );
}
