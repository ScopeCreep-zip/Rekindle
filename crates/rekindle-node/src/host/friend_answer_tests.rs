//! Answering friend requests through the Host (review RV3): an answer to a
//! request that is not there stores nothing (638ae37b), and a cancelled
//! request takes its `FriendRequest` row with it (a3a553d1), in its own
//! commit (RV5 item 3).

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
            depends_on: None,
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

/// Review RV5 item 3: the rows a cancel ends go in the cancel's own
/// commit, so a failure after it (here no outbox is running, as after a
/// crash or a lock right after the commit) leaves no row live for the
/// friendship that ended.
#[tokio::test]
async fn a_cancel_whose_outbox_is_gone_after_the_commit_leaves_no_row() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let (o, bob) = (
        hex::encode(owner),
        hex::encode(Identity::generate().public_key_bytes()),
    );
    let db = host.ctx.db.current().unwrap();
    let (o2, bob2) = (o.clone(), bob.clone());
    db.call(move |conn| {
        rekindle_db::repo::friends::insert_pending_out(conn, &o2, &bob2, "Bob", 1)?;
        for (kind, id) in [("dm", "dm-1"), ("friend_handshake", "request")] {
            let stream = format!("{kind}:{bob2}");
            let row = rekindle_db::repo::outbox::NewRow {
                owner_key: &o2,
                stream: &stream,
                client_id: id,
                kind,
                payload: b"frame",
                next_at: 0,
                deadline: i64::MAX,
                depends_on: None,
            };
            rekindle_db::repo::outbox::enqueue_with_echo(conn, &row, None)?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let loaded = super::session::load(&db, &host.ctx.vault, &o, host.ctx.rng.as_ref(), 0)
        .await
        .unwrap();
    host.ctx.session.write().as_mut().unwrap().friends = loaded.friends;
    host.ctx.outbox.stop();

    friend::cancel(&HostFriends::of(&host.ctx), &bob)
        .await
        .unwrap();
    let left = db
        .call(move |conn| rekindle_db::repo::outbox::prefix_rows(conn, &o, ""))
        .await
        .unwrap();
    // The friend list's own row, committed with the cancel, stays.
    let to_bob: Vec<_> = left
        .iter()
        .filter(|row| row.stream.ends_with(&bob))
        .collect();
    assert!(
        to_bob.is_empty(),
        "every row to Bob went with the cancel: {to_bob:?}"
    );
    assert!(left
        .iter()
        .any(|row| row.stream == rekindle_db::repo::outbox::FRIEND_LIST_STREAM));
    assert_eq!(stored(&host, owner).await, 0);
}

/// GC3 (Linux live): right after two requests were accepted, the New
/// Group picker listed one. The list right after the accepts names every
/// friend with the Host's group-DM eligibility: an accepted friend whose
/// profile we hold can be added; one whose request named no profile, and
/// one still pending our request, are listed with the reason.
#[tokio::test]
async fn the_list_right_after_two_accepts_names_each_friend_with_its_group_dm_eligibility() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let o = hex::encode(owner);
    let key = || hex::encode(Identity::generate().public_key_bytes());
    let (bob, carol, dave) = (key(), key(), key());
    let db = host.ctx.db.current().unwrap();
    let (o2, b2, c2, d2) = (o.clone(), bob.clone(), carol.clone(), dave.clone());
    db.call(move |conn| {
        for (pk, name, profile) in [(&b2, "Bob", "VLD0:bobprofile"), (&c2, "Carol", "")] {
            let request = rekindle_db::repo::pending_requests::Incoming {
                public_key: pk,
                display_name: name,
                message: "hi",
                received_at: 1,
                profile_dht_key: profile,
                route_blob: &[],
                mailbox_dht_key: "",
                invite_id: None,
                reply_inbox_key: "VLD0:reply",
                reply_writer: "VLD0:p:s",
                chain_record_key: "VLD0:chain",
            };
            rekindle_db::repo::pending_requests::upsert(conn, &o2, &request)?;
        }
        rekindle_db::repo::friends::insert_pending_out(conn, &o2, &d2, "Dave", 1).map(drop)
    })
    .await
    .unwrap();
    let loaded = super::session::load(&db, &host.ctx.vault, &o, host.ctx.rng.as_ref(), 0)
        .await
        .unwrap();
    host.ctx.session.write().as_mut().unwrap().friends = loaded.friends;
    let friends = HostFriends::of(&host.ctx);
    friend::accept(&friends, &bob).await.unwrap();
    friend::accept(&friends, &carol).await.unwrap();

    let list = friend::friends(&friends);
    let block = |pk: &str| {
        list.iter()
            .find(|f| f.public_key == pk)
            .unwrap_or_else(|| panic!("{pk} missing from {list:?}"))
            .group_dm_block
            .clone()
    };
    assert_eq!(list.len(), 3, "{list:?}");
    assert_eq!(block(&bob), None);
    assert_eq!(
        block(&carol).as_deref(),
        Some("Their profile is not known yet")
    );
    assert_eq!(
        block(&dave).as_deref(),
        Some("Waiting for them to accept your friend request")
    );
}
