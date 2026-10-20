//! The 1:1 receive pipeline's replay through the Host (plan E2.3, D24):
//! a message decrypted but not applied (a crash between the ratchet commit
//! and the apply) is applied exactly once at the next replay, judged by
//! the friend gate again, and a signed identity rotation moves the friend.

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::{ContentHint, DirectBody, DmContent, DmText, DmTypeId};
use rekindle_db::repo::unprocessed::UnprocessedKey;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::{Caller, HostApi};
use rekindle_transport::operations::direct::replay_unprocessed;

use crate::host::adapters::friends::HostFriends;
use crate::host::{claim, identity, Host, HostConfig, HostStores, TransportSource};

/// An offline host with Ann unlocked; her key.
async fn ann(dir: &tempfile::TempDir) -> (Arc<Host>, [u8; 32]) {
    let data = dir.path().join("data");
    let root = rekindle_db::paths::DataRoot {
        config: dir.path().join("config"),
        logs: dir.path().join("logs"),
        state: data.join("state"),
        data,
    };
    let host = Host::start(HostConfig {
        root: claim(root).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: Arc::new(rekindle_utils::SystemClock),
        rng: Arc::new(rekindle_utils::OsRandom),
        shell: None,
    })
    .await
    .unwrap();
    let created = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    let ann = created.public_key.clone();
    drop(created);
    identity::lock(&host.identity_stores()).await;
    let unlocked = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: ann.clone(),
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(
        matches!(unlocked, rekindle_ipc::protocol::IpcResponse::Ok { .. }),
        "{unlocked:?}"
    );
    (host, hex::decode(ann).unwrap().try_into().unwrap())
}

/// Bob, an accepted friend of Ann's.
async fn befriend(host: &Host, owner: [u8; 32], bob: [u8; 32]) {
    let (owner, bob) = (hex::encode(owner), hex::encode(bob));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::friends::insert_accepted(conn, &owner, &bob, "Bob", 1))
        .await
        .unwrap();
}

/// A decrypted, unapplied message from `from`, as the ratchet records it.
async fn pending(host: &Host, owner: [u8; 32], from: [u8; 32], n: u32, body: &DirectBody) {
    let content = DmContent {
        content_hint: ContentHint::Default,
        sent_at_ms: 1_700_000_000_000,
        payload: body.encode(),
    }
    .encode();
    let key = UnprocessedKey {
        peer_identity: from,
        header_dh: [7; 32],
        header_n: n,
    };
    let type_id = body.type_id().as_u16();
    // Received now: an older row is past the replay's age bound.
    let now = rekindle_utils::timestamp_ms_i64();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::unprocessed::insert(conn, &owner, &key, type_id, &content, now)
        })
        .await
        .unwrap();
}

/// How many messages our conversation with `peer` holds.
async fn history(host: &Host, owner: [u8; 32], peer: [u8; 32]) -> usize {
    let (o, p) = (hex::encode(owner), hex::encode(peer));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::messages::dm_history(conn, &o, &p, 50, None))
        .await
        .unwrap()
        .len()
}

/// How many decrypted messages wait for their apply.
async fn unapplied(host: &Host, owner: [u8; 32]) -> usize {
    let now = rekindle_utils::timestamp_ms_i64();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::unprocessed::take_for_replay(conn, &owner, now))
        .await
        .unwrap()
        .len()
}

fn text(n: u8) -> DirectBody {
    DirectBody::DmText(DmText {
        client_id: [n; 16],
        body: format!("hello {n}"),
        reply_to: None,
        expire_seconds: 0,
    })
}

/// D24: an unapplied message is applied once at the replay, and its row
/// goes; a second replay changes nothing.
#[tokio::test]
async fn an_unapplied_message_is_applied_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    pending(&host, owner, bob, 0, &text(1)).await;

    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    // The delete is queued behind the apply on the database thread.
    assert_eq!(history(&host, owner, bob).await, 1);
    assert_eq!(unapplied(&host, owner).await, 0);

    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    assert_eq!(history(&host, owner, bob).await, 1);
}

/// The replay judges the sender again: a message from someone blocked
/// since it was decrypted is not applied, and its row goes.
#[tokio::test]
async fn the_replay_refuses_a_sender_blocked_since() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    pending(&host, owner, bob, 0, &text(2)).await;
    let (o, b) = (hex::encode(owner), hex::encode(bob));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::blocked::insert(conn, &o, &b, "Bob", 1))
        .await
        .unwrap();

    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    assert_eq!(history(&host, owner, bob).await, 0);
    assert_eq!(unapplied(&host, owner).await, 0);
}

/// A friend's signed rotation (0x0108) moves them, their history and their
/// verification to the new key; the old key's session is gone, and a
/// verified key's change is recorded (it escalates the next review).
#[tokio::test]
async fn a_signed_identity_rotation_moves_the_friend() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let (old, new) = (
        rekindle_crypto::Identity::generate(),
        rekindle_crypto::Identity::generate(),
    );
    befriend(&host, owner, old.public_key_bytes()).await;
    pending(&host, owner, old.public_key_bytes(), 0, &text(3)).await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    let old_key = old.public_key_bytes();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            use rekindle_db::repo::signal_identities as ids;
            ids::rotate(conn, &owner, &[9; 32], &old_key, 1)?;
            ids::mark_verified(conn, &owner, &old_key, 2)
        })
        .await
        .unwrap();

    let rotation =
        rekindle_transport::operations::friend::identity_rotation::rotation(&old, &new, &owner);
    pending(
        &host,
        owner,
        old.public_key_bytes(),
        1,
        &DirectBody::IdentityRotated(rotation),
    )
    .await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;

    let new_hex = hex::encode(new.public_key_bytes());
    let o = hex::encode(owner);
    let friends = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::friends::load_all(conn, &o))
        .await
        .unwrap();
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].public_key, new_hex);
    assert_eq!(history(&host, owner, new.public_key_bytes()).await, 1);
    let new_key = new.public_key_bytes();
    let row = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::signal_identities::get(conn, &owner, &new_key))
        .await
        .unwrap()
        .unwrap();
    assert!(!row.verified);
    assert_eq!(row.changed_from_key, Some(old.public_key_bytes()));
    assert_eq!(DmTypeId::IdentityRotated.as_u16(), 0x0108);
}
