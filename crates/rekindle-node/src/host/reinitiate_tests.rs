//! The last stage of plan E2.5 (B10, ruling Q1): exhausted retry requests
//! hold one `SessionReinitiate` review and send nothing; only the user's
//! commit runs the reset, and Cancel changes nothing.

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_transport::operations::direct::retry_requests;
use rekindle_transport::operations::friend::FriendHost as _;
use rekindle_types::intent::{Fact, IntentClass, IntentKind, IntentProof, IntentReview};
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent};

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, befriend};
use super::Host;

/// Ann with Bob an accepted friend in her projection (the unlock loads it).
async fn ann_and_bob(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, String, String) {
    let (host, owner) = ann(dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let ann_key = hex::encode(owner);
    assert!(matches!(
        host.call(Caller::shell(), IpcRequest::Lock).await,
        IpcResponse::Ok { .. }
    ));
    let unlocked = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: ann_key.clone(),
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(
        !matches!(unlocked, IpcResponse::Error { .. }),
        "{unlocked:?}"
    );
    (host, ann_key, hex::encode(bob))
}

/// Bob's message `id` failed again after its last retry request.
async fn exhaust(host: &Host, owner: &str, peer: &str, id: u8) {
    use rekindle_db::repo::retry_requests as rr;
    let (owner, peer): ([u8; 32], [u8; 32]) = (
        hex::decode(owner).unwrap().try_into().unwrap(),
        hex::decode(peer).unwrap().try_into().unwrap(),
    );
    let window = i64::try_from(rr::PEER_WINDOW.as_millis()).unwrap() * 2;
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            let due = rr::Due {
                msg_id: [id; 32],
                observed_ratchet_key: [9; 32],
            };
            for i in 0..i64::from(rr::MAX_PER_MESSAGE) {
                let at = (i64::from(id) * 16 + i) * window;
                rr::note_failed(conn, &owner, &peer, &due, at)?;
                rr::take_due(conn, &owner, &peer, at)?;
            }
            rr::note_failed(conn, &owner, &peer, &due, 1)
        })
        .await
        .unwrap();
}

fn reinitiate_holds(host: &Host, peer: &str) -> usize {
    let count = std::cell::Cell::new(0);
    host.ctx.intents.retain_held(|held_by, action| {
        if held_by.conn.is_none()
            && matches!(action, IpcRequest::SessionReinitiate { public_key } if public_key == peer)
        {
            count.set(count.get() + 1);
        }
        true
    });
    count.get()
}

async fn reset_requested(host: &Host, owner: &str, peer: &str) -> bool {
    let (owner, peer) = (owner.to_string(), peer.to_string());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::session_reset_requests::take(conn, &owner, &peer))
        .await
        .unwrap()
}

async fn next_review(stream: &mut rekindle_events::journal::EventStream) -> IntentReview {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("an event")
            .expect("the stream");
        if let StreamFrame::Event(sequenced) = frame {
            if let SubscriptionEvent::IntentPending(review) = sequenced.event {
                return review;
            }
        }
    }
}

#[tokio::test]
async fn exhausted_retry_requests_hold_one_reinitiation_and_only_its_commit_resets() {
    for accept in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (host, ann, bob) = ann_and_bob(&dir).await;
        let mut events = host.subscribe(None).await.unwrap();
        let friends = HostFriends::of(&host.ctx);
        // A second exhaustion replaces the first hold: the newer review is
        // the one held.
        let mut review = None;
        for id in [1, 2] {
            exhaust(&host, &ann, &bob, id).await;
            retry_requests::on_caught_up(&friends, &bob).await;
            review = Some(next_review(&mut events).await);
        }
        let review = review.unwrap();
        // One hold per friend; nothing was asked of Bob, nothing re-initiated.
        assert_eq!(reinitiate_holds(&host, &bob), 1);
        assert!(!reset_requested(&host, &ann, &bob).await);
        assert_eq!(review.kind, IntentKind::ReinitiateSession);
        assert_eq!(review.class, IntentClass::TrustChanging);
        assert!(review.facts.contains(&Fact::UnopenedMessagesLost));
        assert!(review.facts.contains(&Fact::SessionKeptUntilReplaced));
        let answer = if accept {
            host.call(
                Caller::shell(),
                IpcRequest::IntentCommit {
                    intent_id: review.intent_id,
                    proof: IntentProof::Acknowledged,
                },
            )
            .await
        } else {
            host.call(
                Caller::shell(),
                IpcRequest::IntentCancel {
                    intent_id: review.intent_id,
                },
            )
            .await
        };
        assert!(!matches!(answer, IpcResponse::Error { .. }), "{answer:?}");
        assert_eq!(reinitiate_holds(&host, &bob), 0);
        // Accept runs the user-started reset; Cancel changes nothing.
        assert_eq!(reset_requested(&host, &ann, &bob).await, accept);
        assert!(friends.stores().incoming_resets.lock().is_empty());
    }
}
