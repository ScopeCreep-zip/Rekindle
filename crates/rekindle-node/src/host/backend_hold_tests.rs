use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::intent::IntentKind;
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent};

use crate::host::reinitiate_tests::ann_and_bob;
use crate::host::{held_reviews_of, Host};

async fn raise(host: &Host, peer: &str) {
    super::raise(
        &host.ctx,
        IpcRequest::SessionReinitiate {
            public_key: peer.to_owned(),
        },
        None,
        IntentKind::ReinitiateSession,
        |a, b| {
            matches!(
                (a, b),
                (
                    IpcRequest::SessionReinitiate { public_key: x },
                    IpcRequest::SessionReinitiate { public_key: y }
                ) if x == y
            )
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn identical_raises_keep_the_id_and_publish_once() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _, bob) = ann_and_bob(&dir).await;
    let before = host.ctx.journal.head();
    raise(&host, &bob).await;
    let first = held_reviews_of(&host.ctx);
    raise(&host, &bob).await;

    let raised: Vec<_> = host
        .ctx
        .journal
        .replay(before)
        .into_iter()
        .filter_map(|frame| {
            let StreamFrame::Event(event) = frame else {
                return None;
            };
            match event.event {
                SubscriptionEvent::IntentPending { review, hold_id } => Some((review, hold_id)),
                _ => None,
            }
        })
        .collect();
    assert_eq!(raised.len(), 1, "one IntentPending across both raises");
    assert_eq!(first.len(), 1);
    assert_eq!(raised[0].1, Some(first[0].hold_id));
    assert_eq!(raised[0].0.intent_id, first[0].review.intent_id);
    assert_eq!(held_reviews_of(&host.ctx), first);
}

#[tokio::test]
async fn a_changed_selection_publishes_the_replacement_set() {
    let dir = tempfile::tempdir().unwrap();
    let (host, ann, bob) = ann_and_bob(&dir).await;
    raise(&host, &bob).await;
    let first = held_reviews_of(&host.ctx);
    let peer = bob.clone();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::friends::set_nickname(conn, &ann, &peer, Some("Bobby"))
        })
        .await
        .unwrap();

    let before = host.ctx.journal.head();
    raise(&host, &bob).await;
    let latest = held_reviews_of(&host.ctx);
    assert_eq!(latest.len(), 1);
    assert_ne!(latest[0].hold_id, first[0].hold_id);
    assert_ne!(latest[0].review.subject, first[0].review.subject);

    let updates: Vec<_> = host
        .ctx
        .journal
        .replay(before)
        .into_iter()
        .filter_map(|frame| {
            let StreamFrame::Event(event) = frame else {
                return None;
            };
            match event.event {
                update @ (SubscriptionEvent::HeldReviews(_)
                | SubscriptionEvent::IntentPending { .. }) => Some(update),
                _ => None,
            }
        })
        .collect();
    let [SubscriptionEvent::HeldReviews(set), SubscriptionEvent::IntentPending { review, hold_id }] =
        updates.as_slice()
    else {
        panic!("expected a held-set update followed by IntentPending, got {updates:?}");
    };
    assert_eq!(set, &latest);
    assert_eq!(*hold_id, Some(latest[0].hold_id));
    assert_eq!(review.intent_id, latest[0].review.intent_id);
}
