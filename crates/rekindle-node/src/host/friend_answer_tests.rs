//! Answering friend requests through the Host (review RV3): an answer to a
//! request that is not there stores nothing (638ae37b), and a cancelled
//! request takes its `FriendRequest` row with it (a3a553d1).

use rekindle_crypto::Identity;
use rekindle_transport::operations::friend::{self, FriendError};

use super::adapters::friends::HostFriends;
use super::direct_tests::ann;
use super::Host;

/// The friends `owner` stores.
async fn stored(host: &Host, owner: [u8; 32]) -> usize {
    let o = hex::encode(owner);
    let db = host.ctx.db.current().unwrap();
    db.call(move |conn| rekindle_db::repo::friends::load_all(conn, &o))
        .await
        .unwrap()
        .len()
}

/// Accepting or rejecting a key that sent no request is `NotFound` and
/// stores no friend (the live bug: `friend accept` stored one).
#[tokio::test]
async fn answering_a_request_that_is_not_there_is_not_found_and_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let stranger = hex::encode(Identity::generate().public_key_bytes());
    let accepted = friend::accept(&friends, &stranger).await;
    assert!(
        matches!(accepted, Err(FriendError::NotFound(_))),
        "{accepted:?}"
    );
    let rejected = friend::reject(&friends, &stranger).await;
    assert!(
        matches!(rejected, Err(FriendError::NotFound(_))),
        "{rejected:?}"
    );
    assert_eq!(stored(&host, owner).await, 0);
}

/// Our request to Bob is pending with its `FriendRequest` row still on the
/// handshake stream (wedged `failed` awaiting Retry): cancelling it ends
/// the row too (live A/B finding 70).
#[tokio::test]
async fn a_cancelled_request_takes_its_request_row_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let (o, bob) = (
        hex::encode(owner),
        hex::encode(Identity::generate().public_key_bytes()),
    );
    let stream = format!("friend_handshake:{bob}");
    let db = host.ctx.db.current().unwrap();
    let (o2, bob2, stream2) = (o.clone(), bob.clone(), stream.clone());
    db.call(move |conn| {
        rekindle_db::repo::friends::insert_pending_out(conn, &o2, &bob2, "Bob", 1)?;
        let row = rekindle_db::repo::outbox::NewRow {
            owner_key: &o2,
            stream: &stream2,
            client_id: "request",
            kind: "friend_handshake",
            payload: b"frame",
            next_at: 0,
            deadline: i64::MAX,
        };
        rekindle_db::repo::outbox::enqueue_with_echo(conn, &row, None)?;
        let wedged = rekindle_db::repo::outbox::Update {
            state: rekindle_types::message::SendState::Failed,
            attempts: 1,
            next_at: 0,
            deadline: i64::MAX,
            wedge_reason: Some("refused"),
        };
        rekindle_db::repo::outbox::update(conn, &o2, "request", &wedged).map(drop)
    })
    .await
    .unwrap();
    let loaded = super::session::load(&db, &host.ctx.vault, &o, host.ctx.rng.as_ref(), 0)
        .await
        .unwrap();
    host.ctx.session.write().as_mut().unwrap().friends = loaded.friends;

    friend::cancel(&HostFriends::of(&host.ctx), &bob)
        .await
        .unwrap();
    let left = db
        .call(move |conn| rekindle_db::repo::outbox::prefix_rows(conn, &o, &stream))
        .await
        .unwrap();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(stored(&host, owner).await, 0);
}
