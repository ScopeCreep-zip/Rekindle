//! The held set (plan E3.2 S7d-keys.10): a review the backend holds is
//! handed to every window that subscribes while it is held, under its
//! publication id, and leaves the set at its commit, cancel, replacement
//! or the lock; the live event carries the same id.

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::intent::{IntentId, IntentKind, IntentProof};
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent};

use super::reinitiate_tests::ann_and_bob;
use super::Host;

fn same(a: &IpcRequest, b: &IpcRequest) -> bool {
    matches!(
        (a, b),
        (
            IpcRequest::SessionReinitiate { .. },
            IpcRequest::SessionReinitiate { .. }
        )
    )
}

/// Raise the reinitiation review on `peer` and return the live event's
/// review id and hold id.
async fn raise(host: &Host, peer: &str) -> (IntentId, Option<IntentId>) {
    let mut events = host.subscribe(None).await.unwrap();
    crate::host::backend_hold::raise(
        &host.ctx,
        IpcRequest::SessionReinitiate {
            public_key: peer.to_owned(),
        },
        None,
        IntentKind::ReinitiateSession,
        same,
    )
    .await
    .unwrap();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .expect("an event")
            .expect("the stream");
        if let StreamFrame::Event(sequenced) = frame {
            if let SubscriptionEvent::IntentPending { review, hold_id } = sequenced.event {
                return (review.intent_id, hold_id);
            }
        }
    }
}

/// The held set a window that subscribes now is handed: (hold id, review id).
fn held(host: &Host) -> Vec<(IntentId, IntentId)> {
    let SubscriptionEvent::HeldReviews(held) = host.held_reviews() else {
        panic!("not the held set");
    };
    held.iter()
        .map(|h| (h.hold_id, h.review.intent_id))
        .collect()
}

#[tokio::test]
async fn a_held_review_is_handed_to_a_later_subscriber_until_it_is_resolved() {
    for commit in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (host, _ann, bob) = ann_and_bob(&dir).await;
        assert!(held(&host).is_empty(), "nothing held yet");

        let (first, hold_id) = raise(&host, &bob).await;
        assert_eq!(hold_id, Some(first), "the live event names its hold");
        assert_eq!(held(&host), vec![(first, first)], "exactly one");
        assert!(host.holds_review(first, first));

        let (latest, _) = raise(&host, &bob).await;
        assert_ne!(latest, first);
        assert!(!host.holds_review(first, first));
        assert!(host.holds_review(latest, latest));
        assert_eq!(
            held(&host),
            vec![(latest, latest)],
            "a replacement supersedes"
        );

        let answer = if commit {
            IpcRequest::IntentCommit {
                intent_id: latest,
                proof: IntentProof::Acknowledged,
            }
        } else {
            IpcRequest::IntentCancel { intent_id: latest }
        };
        let answer = host.call(Caller::shell(), answer).await;
        assert!(!matches!(answer, IpcResponse::Error { .. }), "{answer:?}");
        assert!(held(&host).is_empty(), "resolved (commit: {commit})");
        assert!(!host.holds_review(latest, latest));
    }
}

#[tokio::test]
async fn the_lock_drops_the_held_set() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _ann, bob) = ann_and_bob(&dir).await;
    raise(&host, &bob).await;
    assert_eq!(held(&host).len(), 1);
    assert!(matches!(
        host.call(Caller::shell(), IpcRequest::Lock).await,
        IpcResponse::Ok { .. }
    ));
    assert!(held(&host).is_empty());
}
