//! Media keys on the outbox (plan E2.5): retried until acknowledged,
//! settled unsent once superseded, discarded with the session.

use super::*;
use crate::signaling::media_key_outbox::end_media_key_session;

/// A key to an offline recipient stays queued and the outbox retries it
/// until the recipient, back, acknowledges it.
#[tokio::test(start_paused = true)]
async fn an_offline_recipient_gets_the_key_from_the_outbox_once_back() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice.roster(&[&bob]).await;
    alice.drop_calls.store(3, Ordering::SeqCst);

    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
    )
    .await;
    alice.settle().await;

    assert_eq!(
        alice.take_calls().len(),
        4,
        "three lost, the fourth acknowledged"
    );
    assert_eq!(alice.queued(), 0);
    let ours = alice.sender_keys().send_key(Instant::now());
    assert_eq!(alice.held_by(&bob, ours.index), Some(*ours.secret));
}

/// A queued key settles unsent once the recipient leaves our roster.
#[tokio::test(start_paused = true)]
async fn a_queued_key_settles_unsent_when_the_recipient_leaves() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice.roster(&[&bob]).await;
    alice.drop_calls.store(usize::MAX, Ordering::SeqCst);

    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
    )
    .await;
    alice.settle().await;
    assert_eq!(alice.queued(), 1);
    alice.take_calls();
    alice.transport.lock().await.remove_peer(&bob.id());
    alice.settle().await;

    assert_eq!(alice.queued(), 0);
    assert!(alice.take_calls().is_empty());
}

/// A key a rotation replaced before it was delivered settles without
/// being sent; only the new one goes.
#[tokio::test(start_paused = true)]
async fn a_key_rotated_before_delivery_is_never_sent() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice.roster(&[&bob]).await;
    alice.drop_calls.store(usize::MAX, Ordering::SeqCst);
    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
    )
    .await;
    alice.settle().await;
    let stale = alice.sender_keys().send_key(Instant::now()).index;

    let LeaveRotation::Rotated(fresh) = alice.sender_keys().on_leave(Instant::now()) else {
        panic!("the first leave rotates at once");
    };
    let later = Instant::now() + std::time::Duration::from_secs(3600);
    assert_eq!(alice.sender_keys().send_key(later).index, fresh.index);
    assert!(!alice.sender_keys().still_current(stale));
    queue_delivery(&alice.deps(), COMMUNITY, CHANNEL, &bob.id(), &fresh).await;
    alice.take_calls();
    alice.drop_calls.store(0, Ordering::SeqCst);
    alice.settle().await;

    assert_eq!(alice.take_calls(), vec![(bob.id(), fresh.index)]);
    assert_eq!(alice.held_by(&bob, stale), None);
    assert_eq!(alice.queued(), 0);
}

/// The session's end discards its queued keys with its keyring.
#[tokio::test(start_paused = true)]
async fn the_session_end_discards_its_queued_keys() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice.roster(&[&bob]).await;
    alice.drop_calls.store(usize::MAX, Ordering::SeqCst);
    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
    )
    .await;
    alice.settle().await;
    assert_eq!(alice.queued(), 1);

    end_media_key_session(&alice.keys, &*alice.outbox, COMMUNITY, CHANNEL).await;
    assert_eq!(alice.queued(), 0);
}
