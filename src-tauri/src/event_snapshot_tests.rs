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
