//! The 1:1 receive pipeline's edges through the Host (plan audit G10, B7):
//! an ephemeral kind leaves no `unprocessed` row, a row whose body does not
//! decode goes (on receive and at the replay), and an accepted friend with
//! no session is opened again or fails visibly.

use rekindle_codec::capnp_codec::direct::{
    ContentHint, DirectBody, DmContent, DmFrame, DmTypeId, FrameBody, FriendAccept, Typing,
};
use rekindle_crypto::signal::prekey_record::base_key_hash;
use rekindle_crypto::Identity;
use rekindle_db::ratchet::PeerAddress;
use rekindle_db::repo::unprocessed::UnprocessedKey;
use rekindle_transport::operations::direct::{receive, replay_unprocessed};
use rekindle_transport::operations::dm::{frame, wire};
use rekindle_transport::operations::friend::FriendHost;

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, befriend, call, history, now_ms, sealed_request, text, unapplied};
use super::prekey_tests::{bundle_of, own_bundle, rejection};
use super::Host;

/// `peer` an accepted friend of `owner`'s, in the store and the projection
/// (as an unlock loads it).
async fn accepted(host: &Host, owner: [u8; 32], peer: [u8; 32]) {
    befriend(host, owner, peer).await;
    let db = host.ctx.db.current().unwrap();
    let loaded = super::session::load(
        &db,
        &host.ctx.vault,
        &hex::encode(owner),
        host.ctx.rng.as_ref(),
        0,
    )
    .await
    .unwrap();
    let mut session = host.ctx.session.write();
    let session = session.as_mut().unwrap();
    session.friends = loaded.friends;
    session.presence_key = loaded.presence_key;
}

/// The events already in `stream`, as text.
async fn drain(stream: &mut rekindle_events::EventStream) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await
    {
        seen.push(format!("{frame:?}"));
    }
    seen
}

/// How many of `events` mention `needle`.
fn mentions(events: &[String], needle: &str) -> usize {
    events.iter().filter(|e| e.contains(needle)).count()
}

/// The rows `owner`'s outbox holds.
async fn queued(host: &Host, owner: [u8; 32]) -> i64 {
    let owner = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::outbox::unsettled_count(conn, &owner))
        .await
        .unwrap()
}

/// Ann and Bob, each the other's accepted friend, with the session Bob
/// opened by a first message (Ann accepted it); Bob's host and key.
async fn session_pair(
    dir: &tempfile::TempDir,
    theirs: &tempfile::TempDir,
) -> (
    std::sync::Arc<Host>,
    [u8; 32],
    std::sync::Arc<Host>,
    [u8; 32],
) {
    let (host, owner) = ann(dir).await;
    let (other, bob) = ann(theirs).await;
    accepted(&host, owner, bob).await;
    accepted(&other, bob, owner).await;
    let bundle = own_bundle(&host, owner).await;
    let sent = frame::initiate_frame(
        &HostFriends::of(&other.ctx),
        &hex::encode(owner),
        bundle,
        &text(1),
    )
    .await
    .unwrap();
    receive(&HostFriends::of(&host.ctx), &wire(&sent).unwrap()).await;
    assert_eq!(history(&host, owner, bob).await, 1, "the session opened");
    (host, owner, other, bob)
}

/// G10: Ann's Typing decrypts at Bob's under the session, with no
/// `unprocessed` row, and neither side queues it.
#[tokio::test]
async fn an_ephemeral_kind_leaves_no_unprocessed_row() {
    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner, other, bob) = session_pair(&dir, &theirs).await;
    let mut events = other.ctx.journal.subscribe(None);

    let typing = DirectBody::Typing(Typing { typing: true });
    let sent = frame::ratchet_frame(
        &HostFriends::of(&host.ctx),
        &hex::encode(bob),
        &typing,
        ContentHint::Default,
    )
    .await
    .unwrap();
    assert!(matches!(sent.body, FrameBody::Ratchet(_)));
    receive(&HostFriends::of(&other.ctx), &wire(&sent).unwrap()).await;

    assert!(
        mentions(&drain(&mut events).await, "Typing") > 0,
        "it decrypted"
    );
    assert_eq!(unapplied(&other, bob).await, 0);
    assert_eq!(
        (queued(&host, owner).await, queued(&other, bob).await),
        (0, 0)
    );
}

/// Bob's ratchet message to Ann whose content is `content` (any bytes).
async fn ratcheted(other: &Host, owner: [u8; 32], content: &[u8]) -> Vec<u8> {
    let local = HostFriends::of(&other.ctx).local_party().unwrap();
    let message = rekindle_db::ratchet::encrypt_and_record_outbound(
        &other.ctx.db.current().unwrap(),
        &local,
        &PeerAddress::sole_device(owner),
        content,
        &frame::ad_extra(DmTypeId::DmText),
    )
    .await
    .unwrap();
    wire(&DmFrame {
        type_id: DmTypeId::DmText,
        sender: local.owner(),
        body: FrameBody::Ratchet(frame::ratchet_body(message)),
    })
    .unwrap()
}

/// A text's content whose body is not a text.
fn undecodable_body() -> Vec<u8> {
    DmContent {
        stream_seq: 0,
        content_hint: ContentHint::Default,
        sent_at_ms: 1_700_000_000_000,
        payload: vec![0xFF; 3],
    }
    .encode()
}

/// A message whose content or body does not decode is dropped on receive
/// and its `unprocessed` row goes.
#[tokio::test]
async fn an_undecodable_message_leaves_no_row_on_receive() {
    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner, other, bob) = session_pair(&dir, &theirs).await;
    let friends = HostFriends::of(&host.ctx);
    for content in [undecodable_body(), b"not content".to_vec()] {
        receive(&friends, &ratcheted(&other, owner, &content).await).await;
    }
    assert_eq!(unapplied(&host, owner).await, 0);
    assert_eq!(history(&host, owner, bob).await, 1, "only the first text");
}

/// The same at the startup replay: a stored row that does not decode goes.
#[tokio::test]
async fn an_undecodable_row_is_deleted_at_the_replay() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let now = rekindle_utils::timestamp_ms_i64();
    let type_id = DmTypeId::DmText.as_u16();
    for (n, content) in [undecodable_body(), b"not content".to_vec()]
        .into_iter()
        .enumerate()
    {
        let key = UnprocessedKey {
            peer_identity: bob,
            header_dh: [7; 32],
            header_n: u32::try_from(n).unwrap(),
        };
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
    assert_eq!(unapplied(&host, owner).await, 2);

    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    assert_eq!(unapplied(&host, owner).await, 0);
    assert_eq!(history(&host, owner, bob).await, 0);
}

/// B7: a request from an accepted friend we hold no session with re-runs
/// our first contact (here it cannot open: no profile, so it fails
/// visibly); with an established session it is a retry and does nothing.
#[tokio::test]
async fn a_request_from_an_accepted_friend_reopens_only_a_missing_session() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = Identity::generate();
    accepted(&host, owner, bob.public_key_bytes()).await;
    let mut events = host.ctx.journal.subscribe(None);
    let request = sealed_request(&bob, owner, [8; 16], now_ms());
    assert!(call(&host, &request).await.is_empty(), "receipted");
    assert_eq!(mentions(&drain(&mut events).await, "HandshakeFailed"), 1);

    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner, other, _bob) = session_pair(&dir, &theirs).await;
    let bob = HostFriends::of(&other.ctx).local_party().unwrap().identity;
    let mut events = host.ctx.journal.subscribe(None);
    let request = sealed_request(&bob, owner, [9; 16], now_ms());
    assert!(call(&host, &request).await.is_empty(), "receipted");
    assert_eq!(
        mentions(&drain(&mut events).await, "Handshake"),
        0,
        "nothing reopened"
    );
}

/// B7: Bob's genuine rejection of an init we hold no first contact for
/// (an accepted friend's, lost with a restart) fails visibly, with no
/// retry.
#[tokio::test]
async fn a_rejection_with_no_first_contact_fails_visibly() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let bob = Identity::generate();
    accepted(&host, owner, bob.public_key_bytes()).await;
    let accept = DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: String::new(),
        route_blob: Vec::new(),
        mailbox_dht_key: String::new(),
    });
    let sent = frame::initiate_frame(
        &friends,
        &hex::encode(bob.public_key_bytes()),
        bundle_of(&bob),
        &accept,
    )
    .await
    .unwrap();
    let FrameBody::PreKeyInit(init) = sent.body else {
        panic!("a first contact is a PreKeyInit");
    };
    // The first contact `initiate_frame` recorded is lost, as with a restart.
    let bob_hex = hex::encode(bob.public_key_bytes());
    assert!(friends
        .stores()
        .first_contacts
        .lock()
        .remove(&bob_hex)
        .is_some());
    let mut events = host.ctx.journal.subscribe(None);
    let hash = base_key_hash(&init.base_key);
    receive(&friends, &rejection(&bob, owner, &init, hash)).await;
    let events = drain(&mut events).await;
    assert_eq!(
        mentions(&events, "refused the keys of our secure session"),
        1,
        "{events:?}"
    );
    assert_eq!(mentions(&events, "HandshakeRetrying"), 0);
}

/// Plan E2.5: an accepted friend's `FriendsKey` is stored and opens their
/// sealed STATUS; removing them rotates our key to epoch 2 and forgets
/// theirs, in the projection and the store.
#[tokio::test]
async fn a_friends_key_is_stored_and_a_removal_rotates_ours() {
    use rekindle_codec::capnp_codec::direct::FriendsKey;
    use rekindle_secrets::friend_presence::{open, seal};
    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner, other, bob) = session_pair(&dir, &theirs).await;
    let bob_hex = hex::encode(bob);
    let bob_key = other
        .ctx
        .session
        .read()
        .as_ref()
        .unwrap()
        .presence_key
        .clone()
        .unwrap();
    let body = DirectBody::FriendsKey(FriendsKey {
        epoch: bob_key.epoch(),
        key: *bob_key.as_bytes(),
    });
    let sent = frame::ratchet_frame(
        &HostFriends::of(&other.ctx),
        &hex::encode(owner),
        &body,
        ContentHint::Default,
    )
    .await
    .unwrap();
    receive(&HostFriends::of(&host.ctx), &wire(&sent).unwrap()).await;
    let stored = host
        .ctx
        .session
        .read()
        .as_ref()
        .unwrap()
        .friend_presence_keys
        .get(&bob_hex)
        .cloned()
        .expect("stored");
    let status = seal(&bob_key, &bob_hex, 2, &[0]).unwrap();
    assert_eq!(open(&stored, &bob_hex, 2, &status).unwrap(), [0]);

    rekindle_transport::operations::friend::remove(&HostFriends::of(&host.ctx), &bob_hex)
        .await
        .unwrap();
    {
        let session = host.ctx.session.read();
        let session = session.as_ref().unwrap();
        assert_eq!(session.presence_key.as_ref().unwrap().epoch(), 2);
        assert!(!session.friend_presence_keys.contains_key(&bob_hex));
    }
    let o = hex::encode(owner);
    let (current, theirs_kept) = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            Ok((
                rekindle_db::repo::presence_keys::current(conn, &o)?,
                rekindle_db::repo::friends::presence_keys(conn, &o)?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(current.map(|(epoch, _)| epoch), Some(2));
    assert!(
        theirs_kept.is_empty(),
        "the removed friend's key is cleared"
    );
}
