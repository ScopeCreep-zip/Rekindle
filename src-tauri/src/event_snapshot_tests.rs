//! A window's snapshot on subscribe and on resume (plan E3.2 S7d-keys.10).

use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::{resume_for, standing_for};
use crate::test_support::TestHost;

fn alert() -> SubscriptionEvent {
    SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
        id: "01900000-0000-7000-8000-0000000000a1".into(),
        title: "Audit chain broken".into(),
        body: "b".into(),
        subject: None,
    })
}

/// (cursor present, standing, payload) of each envelope.
fn shape(envelopes: &[crate::event_router::OutboundEnvelope]) -> Vec<(bool, bool, String)> {
    envelopes
        .iter()
        .map(|e| (e.cursor.is_some(), e.standing, e.payload.to_string()))
        .collect()
}

#[tokio::test]
async fn a_resume_is_handed_the_standing_alert_and_the_held_set_after_its_replay() {
    let t = TestHost::start().await;
    let at = t.host.publish_standing(alert());
    let alert_json = serde_json::to_value(alert()).unwrap().to_string();
    let held_json = r#"{"heldReviews":[]}"#.to_owned();

    let first = standing_for(&t.host, crate::window_labels::BUDDY_LIST);
    assert_eq!(
        shape(&first),
        vec![
            (true, true, alert_json.clone()),
            (false, true, held_json.clone())
        ]
    );

    let resumed = resume_for(&t.host, crate::window_labels::BUDDY_LIST, at);
    assert_eq!(
        shape(&resumed),
        vec![
            (
                true,
                false,
                format!(r#"{{"resumed":{}}}"#, serde_json::to_string(&at).unwrap())
            ),
            (true, true, alert_json),
            (false, true, held_json),
        ],
        "the replay, then the same snapshot with the alert's id"
    );
    assert_eq!(resumed[1].cursor, Some(at));
    t.shutdown().await;
}

#[tokio::test]
async fn a_resume_reconstructs_a_review_raised_after_the_window_went_away() {
    use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
    use rekindle_ipc::{Caller, HostApi};
    use rekindle_node::host::test_fixtures::{create_ann, hold_reinitiation};

    let t = TestHost::start().await;
    create_ann(&t.host).await;
    let at = t.host.publish_standing(alert());

    hold_reinitiation(&t.host, "ab".repeat(32)).await;
    let snapshot = t.host.held_reviews();
    let SubscriptionEvent::HeldReviews(held) = &snapshot else {
        panic!("expected the held set");
    };
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].hold_id, held[0].review.intent_id);

    let expected = serde_json::to_value(snapshot.clone()).unwrap();
    let resumed = resume_for(&t.host, crate::window_labels::BUDDY_LIST, at);
    let envelope = resumed.last().expect("held snapshot after replay");
    assert!(envelope.standing);
    assert!(envelope.cursor.is_none());
    assert_eq!(envelope.payload, expected);

    assert!(matches!(
        t.host
            .call(
                Caller::shell(),
                IpcRequest::IntentCancel {
                    intent_id: held[0].review.intent_id
                }
            )
            .await,
        IpcResponse::Ok { .. }
    ));
    let resumed = resume_for(&t.host, crate::window_labels::BUDDY_LIST, at);
    assert_eq!(
        resumed.last().unwrap().payload,
        serde_json::json!({"heldReviews": []})
    );
    t.shutdown().await;
}
