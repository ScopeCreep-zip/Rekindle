//! The 1:1 receive pipeline's replay through the Host (plan E2.3, D24):
//! a message decrypted but not applied (a crash between the ratchet commit
//! and the apply) is applied exactly once at the next replay, judged by
//! the friend gate again, and a signed identity rotation moves the friend.

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::{
    ContentHint, DirectBody, DmContent, DmText, DmTypeId, FriendAccept,
};
use rekindle_db::repo::unprocessed::UnprocessedKey;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_transport::operations::direct::{self, replay_unprocessed};
use rekindle_transport::operations::friend::FriendHost;
use rekindle_types::intent::{Fact, IntentProof, ProofRequired};

use crate::host::adapters::friends::HostFriends;
use crate::host::{claim, identity, Host, HostConfig, HostStores, TransportSource};

/// An offline host with Ann unlocked; her key.
pub(super) async fn ann(dir: &tempfile::TempDir) -> (Arc<Host>, [u8; 32]) {
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
pub(super) async fn befriend(host: &Host, owner: [u8; 32], bob: [u8; 32]) {
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
pub(super) async fn history(host: &Host, owner: [u8; 32], peer: [u8; 32]) -> usize {
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
pub(super) async fn unapplied(host: &Host, owner: [u8; 32]) -> usize {
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

pub(super) fn text(n: u8) -> DirectBody {
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

/// A verified friend's signed rotation (0x0108) is held: nothing moves
/// until the user commits its review (owner ruling R1, threat model S1).
/// Committing moves them, their history and their verification to the new
/// key (the old key's session is gone, the verified key's change is
/// recorded); declining leaves the old friend intact.
#[tokio::test]
async fn a_signed_identity_rotation_moves_the_friend_only_when_accepted() {
    for accept in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (host, owner) = ann(&dir).await;
        let (old, new) = (
            rekindle_crypto::Identity::generate(),
            rekindle_crypto::Identity::generate(),
        );
        rotated_to(&host, owner, &old, &new).await;
        // Held: the friend, their history and their verification stay.
        assert_eq!(
            friend_keys(&host, owner).await,
            [hex::encode(old.public_key_bytes())]
        );
        assert_eq!(history(&host, owner, old.public_key_bytes()).await, 1);
        assert!(
            identity_row(&host, owner, old.public_key_bytes())
                .await
                .unwrap()
                .verified
        );
        assert!(identity_row(&host, owner, new.public_key_bytes())
            .await
            .is_none());

        let request = if accept {
            IpcRequest::IdentityRotationAccept {
                public_key: hex::encode(old.public_key_bytes()),
            }
        } else {
            IpcRequest::IdentityRotationDecline {
                public_key: hex::encode(old.public_key_bytes()),
            }
        };
        let IpcResponse::IntentRequired(review) = host.call(Caller::shell(), request).await else {
            panic!("the rotation's answer was not held for review");
        };
        assert!(review.facts.iter().any(|f| matches!(
            f,
            Fact::SafetyNumberChanged { old_safety_number, new_safety_number }
                if old_safety_number != new_safety_number
        )));
        if accept {
            assert_eq!(
                review.class,
                rekindle_types::intent::IntentClass::TrustChanging
            );
        }
        let committed = host
            .call(
                Caller::shell(),
                IpcRequest::IntentCommit {
                    intent_id: review.intent_id,
                    proof: if review.class.proof() == ProofRequired::Acknowledge {
                        IntentProof::Acknowledged
                    } else {
                        IntentProof::Commit
                    },
                },
            )
            .await;
        assert!(matches!(committed, IpcResponse::Ok { .. }), "{committed:?}");

        let (kept, gone) = if accept { (&new, &old) } else { (&old, &new) };
        assert_eq!(
            friend_keys(&host, owner).await,
            [hex::encode(kept.public_key_bytes())]
        );
        assert_eq!(history(&host, owner, kept.public_key_bytes()).await, 1);
        assert!(identity_row(&host, owner, gone.public_key_bytes())
            .await
            .is_none());
        let row = identity_row(&host, owner, kept.public_key_bytes())
            .await
            .unwrap();
        if accept {
            assert!(!row.verified);
            assert_eq!(row.changed_from_key, Some(old.public_key_bytes()));
        } else {
            assert!(row.verified);
        }
        assert!(HostFriends::of(&host.ctx)
            .stores()
            .pending_rotations
            .lock()
            .is_empty());
    }
    assert_eq!(DmTypeId::IdentityRotated.as_u16(), 0x0108);
}

/// Bob as `old`, verified, with one message, then his rotation to `new`
/// applied from the ratchet's unprocessed rows.
async fn rotated_to(
    host: &Host,
    owner: [u8; 32],
    old: &rekindle_crypto::Identity,
    new: &rekindle_crypto::Identity,
) {
    befriend(host, owner, old.public_key_bytes()).await;
    pending(host, owner, old.public_key_bytes(), 0, &text(3)).await;
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
        rekindle_transport::operations::friend::identity_rotation::rotation(old, new, &owner);
    pending(
        host,
        owner,
        old.public_key_bytes(),
        1,
        &DirectBody::IdentityRotated(rotation),
    )
    .await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
}

/// Ann's friends' keys.
async fn friend_keys(host: &Host, owner: [u8; 32]) -> Vec<String> {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::friends::load_all(conn, &o))
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.public_key)
        .collect()
}

/// Ann's `signal_identities` row for `peer`.
async fn identity_row(
    host: &Host,
    owner: [u8; 32],
    peer: [u8; 32],
) -> Option<rekindle_db::repo::signal_identities::SignalIdentity> {
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::signal_identities::get(conn, &owner, &peer))
        .await
        .unwrap()
}

/// D24, audit G5: a FriendAccept whose store fails is not processed; its
/// row stays for the next replay, which applies it once the store works.
#[tokio::test]
async fn a_friend_accept_whose_store_fails_keeps_its_row() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let accept = DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: String::new(),
        route_blob: Vec::new(),
        mailbox_dht_key: String::new(),
    });
    pending(&host, owner, bob, 0, &accept).await;
    let db = host.ctx.db.current().unwrap();
    // Every write to a friend's row fails on this connection.
    db.call(|conn| {
        conn.execute_batch(
            "CREATE TEMP TRIGGER refuse_friend_writes BEFORE UPDATE ON main.friends \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
    })
    .await
    .unwrap();

    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    assert_eq!(unapplied(&host, owner).await, 1, "the row is kept");

    db.call(|conn| conn.execute_batch("DROP TRIGGER temp.refuse_friend_writes;"))
        .await
        .unwrap();
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    assert_eq!(
        unapplied(&host, owner).await,
        0,
        "applied once the store works"
    );
}

/// Now, on the host's clock (the system's).
fn now_ms() -> u64 {
    rekindle_utils::time::Clock::now_ms(&rekindle_utils::SystemClock)
}

/// A friend request from `from` to `to`, sealed under `nonce` at `sent_at`.
fn sealed_request(
    from: &rekindle_crypto::Identity,
    to: [u8; 32],
    nonce: [u8; 16],
    sent_at: u64,
) -> rekindle_codec::capnp_codec::direct::DmFrame {
    let body = DirectBody::FriendRequest(rekindle_codec::capnp_codec::direct::FriendRequest {
        display_name: "Bob".into(),
        message: "hi".into(),
        profile_dht_key: format!("VLD0:{}", "A".repeat(43)),
        route_blob: Vec::new(),
        mailbox_dht_key: format!("VLD0:{}", "B".repeat(43)),
        invite_id: None,
    });
    rekindle_transport::operations::dm::frame::seal_presession(from, &to, &body, nonce, sent_at)
        .unwrap()
}

/// `frame` by `app_call` to `host`; its reply.
async fn call(host: &Host, frame: &rekindle_codec::capnp_codec::direct::DmFrame) -> Vec<u8> {
    let datagram = rekindle_transport::operations::dm::wire(frame).unwrap();
    reply(host, &datagram).await
}

/// `datagram` by `app_call` to `host`; its reply.
async fn reply(host: &Host, datagram: &[u8]) -> Vec<u8> {
    match host.receive_direct_call(datagram).await {
        direct::Called::Answered(reply) => reply,
        direct::Called::Unported(opened) => panic!("unported {:?}", opened.type_id),
    }
}

/// The friend requests Ann holds.
async fn requests(host: &Host, owner: [u8; 32]) -> usize {
    let owner = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::pending_requests::list(conn, &owner))
        .await
        .unwrap()
        .len()
}

/// How many `RequestReceived` events are already in `stream`.
async fn received_events(stream: &mut rekindle_events::EventStream) -> usize {
    let mut count = 0;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await
    {
        count += usize::from(format!("{frame:?}").contains("RequestReceived"));
    }
    count
}

/// Audit G3: a call that does not open is answered with a NAK, never the
/// empty receipt — one case per class of drop.
#[tokio::test]
async fn every_dropped_call_is_answered_with_a_nak() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let (bob, mallory) = (
        rekindle_crypto::Identity::generate(),
        rekindle_crypto::Identity::generate(),
    );
    let nak = direct::NAK.to_vec();

    // Undecodable: untagged, and a tagged frame that is not one.
    assert_eq!(reply(&host, b"hello").await, nak);
    assert_eq!(reply(&host, b"Dnot a frame").await, nak);

    // Not admitted: a stranger may not ask to reset a session.
    let reset =
        DirectBody::SessionResetRequest(rekindle_codec::capnp_codec::direct::SessionResetRequest {
            reason: String::new(),
        });
    let frame = rekindle_transport::operations::dm::frame::seal_presession(
        &bob,
        &owner,
        &reset,
        [1; 16],
        now_ms(),
    )
    .unwrap();
    assert_eq!(call(&host, &frame).await, nak);

    // A bad signature: Mallory's frame in Bob's name.
    let mut forged = sealed_request(&mallory, owner, [2; 16], now_ms());
    forged.sender = bob.public_key_bytes();
    assert_eq!(call(&host, &forged).await, nak);

    // Stale: sent long outside the freshness window.
    let stale = sealed_request(&bob, owner, [3; 16], now_ms() - 3_600_000);
    assert_eq!(call(&host, &stale).await, nak);

    assert_eq!(requests(&host, owner).await, 0, "nothing was stored");
}

/// Audit G4: a sealed request is stored before its receipt; a copy (the
/// same frame, or re-sealed under its nonce) is a duplicate, receipted
/// again; the same request under a new nonce stores nothing new. One row,
/// one event.
#[tokio::test]
async fn a_receipted_request_is_stored_once() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let bob = rekindle_crypto::Identity::generate();

    let first = sealed_request(&bob, owner, [4; 16], now_ms());
    assert!(call(&host, &first).await.is_empty(), "receipted");
    assert_eq!(requests(&host, owner).await, 1, "stored before the answer");

    assert!(call(&host, &first).await.is_empty(), "a copy is receipted");
    let resealed = sealed_request(&bob, owner, [4; 16], now_ms() + 1);
    assert!(call(&host, &resealed).await.is_empty(), "so is a re-seal");
    let again = sealed_request(&bob, owner, [5; 16], now_ms());
    assert!(
        call(&host, &again).await.is_empty(),
        "the same request again"
    );

    assert_eq!(requests(&host, owner).await, 1);
    assert_eq!(received_events(&mut events).await, 1, "one event");
}

/// Audit G4: a request whose store fails is refused, and its retry under
/// the same nonce goes through once the store works.
#[tokio::test]
async fn a_request_whose_store_fails_is_refused_and_its_retry_lands() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate();
    let db = host.ctx.db.current().unwrap();
    db.call(|conn| {
        conn.execute_batch(
            "CREATE TEMP TRIGGER refuse_requests BEFORE INSERT ON main.pending_friend_requests \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
    })
    .await
    .unwrap();
    let frame = sealed_request(&bob, owner, [6; 16], now_ms());
    assert_eq!(call(&host, &frame).await, direct::NAK.to_vec());

    db.call(|conn| conn.execute_batch("DROP TRIGGER temp.refuse_requests;"))
        .await
        .unwrap();
    assert!(call(&host, &frame).await.is_empty(), "the retry lands");
    assert_eq!(requests(&host, owner).await, 1);
}
