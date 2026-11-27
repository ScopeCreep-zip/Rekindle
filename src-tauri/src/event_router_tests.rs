use super::*;

fn channel() -> Channel<OutboundEnvelope> {
    Channel::new(|_| Ok(()))
}

#[test]
fn latest_register_wins_and_destroy_removes() {
    let router = WebviewRouter::default();
    router.register("chat-1".into(), channel(), Vec::new);
    router.register("chat-1".into(), channel(), Vec::new);
    router.register("buddy-list".into(), channel(), Vec::new);
    assert_eq!(
        router.labels(),
        vec!["buddy-list".to_owned(), "chat-1".to_owned()]
    );
    router.remove("chat-1");
    assert_eq!(router.labels(), vec!["buddy-list".to_owned()]);
}

#[test]
fn audience_matches_labels_and_kinds() {
    let audience =
        Audience::labels(["chat-0123456789abcdef".to_owned()]).with_kinds(&[WindowKind::Community]);
    assert!(audience.includes("chat-0123456789abcdef"));
    assert!(!audience.includes("chat-fedcba9876543210"));
    assert!(audience.includes("community-browser"));
    assert!(!audience.includes("login"));
    assert!(Audience::post_auth().includes("settings"));
    assert!(!Audience::post_auth().includes("login"));
}

#[test]
fn a_resolved_backend_review_cannot_follow_the_new_windows_empty_snapshot() {
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};
    use tauri::ipc::InvokeResponseBody;

    let router = WebviewRouter::default();
    let still_held = Cell::new(true);
    let queued = OutboundEnvelope {
        cursor: None,
        channel: "intent-event",
        payload: serde_json::json!({"intentPending": {"holdId": "queued"}}),
        standing: false,
    };
    still_held.set(false);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = Arc::clone(&seen);
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("event envelope is JSON");
        };
        capture.lock().unwrap().push(json);
        Ok(())
    });
    router.register("buddy-list".into(), channel, || {
        assert!(
            router.subs.try_read().is_none(),
            "snapshot is under the dispatch lock"
        );
        assert!(!still_held.get());
        vec![OutboundEnvelope {
            cursor: None,
            channel: "intent-event",
            payload: serde_json::json!({"heldReviews": []}),
            standing: true,
        }]
    });
    router.deliver_if(&queued, &Audience::post_auth(), || {
        assert!(
            router.subs.try_write().is_none(),
            "delivery checks under the dispatch lock"
        );
        still_held.get()
    });

    let received = seen.lock().unwrap();
    assert_eq!(received.len(), 1);
    let envelope: serde_json::Value = serde_json::from_str(&received[0]).unwrap();
    assert_eq!(envelope["payload"], serde_json::json!({"heldReviews": []}));
}
