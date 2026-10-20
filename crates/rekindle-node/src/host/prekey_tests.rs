//! First contact through the Host (plan E2.4 Verify). A forged
//! `PreKeyRejected` changes nothing (B6): Ann's first contact to Bob is a
//! PreKeyInit nobody has answered, and only Bob's signed rejection naming
//! its base key discards it. One sealed by another friend, even naming the
//! right base key, and one from Bob naming a base key Ann never sent, both
//! leave the pending init as it was. And a request is never sent before
//! the profile it names is published.

use rekindle_codec::capnp_codec::direct::{
    DirectBody, FrameBody, FriendAccept, PreKeyInitBody, PreKeyRejected,
};
use rekindle_crypto::signal::prekey_record::{base_key_hash, mint_set};
use rekindle_crypto::signal::PreKeyBundle;
use rekindle_crypto::Identity;
use rekindle_db::ratchet::{discard_pending_init, PeerAddress};
use rekindle_transport::operations::direct::receive;
use rekindle_transport::operations::dm::{frame, wire};

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, befriend};
use super::Host;

/// Bob's bundle from his record's head and first slot, as Ann's pick
/// assembles it.
fn bundle_of(bob: &Identity) -> PreKeyBundle {
    let set = mint_set(
        bob,
        &rekindle_utils::OsRandom,
        rekindle_utils::timestamp_ms_i64(),
    );
    PreKeyBundle::from_record(
        &bob.public_key_bytes(),
        &set.head.record,
        Some(&set.slots[0].record),
    )
    .unwrap()
}

/// `from`'s sealed rejection of `init`, naming `hash` as its base key's,
/// as the datagram that reaches Ann.
fn rejection(from: &Identity, ann: [u8; 32], init: &PreKeyInitBody, hash: [u8; 32]) -> Vec<u8> {
    let body = DirectBody::PreKeyRejected(PreKeyRejected {
        signed_prekey_id: init.signed_prekey_id,
        one_time_prekey_id: init.one_time_prekey_id,
        pq_prekey: init.pq_prekey,
        base_key_hash: hash,
    });
    let mut nonce = [0u8; 16];
    rekindle_utils::RandomSource::fill(&rekindle_utils::OsRandom, &mut nonce);
    let sealed =
        frame::seal_presession(from, &ann, &body, nonce, rekindle_utils::timestamp_ms()).unwrap();
    wire(&sealed).unwrap()
}

/// Whether Ann's init to Bob is still unanswered: the discard B6 makes
/// for a genuine rejection still finds it (and makes it).
async fn discard(host: &Host, owner: [u8; 32], bob: [u8; 32], hash: [u8; 32]) -> bool {
    let db = host.ctx.db.current().unwrap();
    let now = rekindle_utils::timestamp_ms_i64();
    discard_pending_init(&db, &owner, &PeerAddress::sole_device(bob), hash, now)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_forged_prekey_rejection_leaves_the_pending_init() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let (bob, mallory) = (Identity::generate(), Identity::generate());
    befriend(&host, owner, bob.public_key_bytes()).await;
    befriend(&host, owner, mallory.public_key_bytes()).await;

    let bob_hex = hex::encode(bob.public_key_bytes());
    let first_contact = || async {
        let accept = DirectBody::FriendAccept(FriendAccept {
            profile_dht_key: String::new(),
            route_blob: Vec::new(),
            mailbox_dht_key: String::new(),
        });
        let sent = frame::initiate_frame(&friends, &bob_hex, bundle_of(&bob), &accept)
            .await
            .unwrap();
        let FrameBody::PreKeyInit(init) = sent.body else {
            panic!("a first contact is a PreKeyInit");
        };
        init
    };

    // Another friend's rejection naming the right base key, and Bob's
    // naming one Ann never sent: the init is still pending after both.
    let init = first_contact().await;
    let hash = base_key_hash(&init.base_key);
    receive(&friends, &rejection(&mallory, owner, &init, hash)).await;
    receive(&friends, &rejection(&bob, owner, &init, [0x5A; 32])).await;
    assert!(
        discard(&host, owner, bob.public_key_bytes(), hash).await,
        "the forged rejections left the init pending"
    );

    // Bob's genuine rejection of the next init does discard it, so the
    // forgeries were refused for what they are.
    let init = first_contact().await;
    let hash = base_key_hash(&init.base_key);
    receive(&friends, &rejection(&bob, owner, &init, hash)).await;
    assert!(
        !discard(&host, owner, bob.public_key_bytes(), hash).await,
        "the genuine rejection discarded the pending init"
    );
}

/// A friend request names our profile, whose device list leads to our
/// prekey record: before the own-records stage has published it (here:
/// never, on a host with no network), adding a friend waits for it, then
/// refuses (`NotReady`) and stores nothing, rather than send a request the
/// peer could never open a session from.
#[tokio::test(start_paused = true)]
async fn adding_a_friend_waits_for_our_published_profile() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let bob = hex::encode(Identity::generate().public_key_bytes());

    let added = rekindle_transport::operations::friend::add(&friends, &bob, "Bob", "hi").await;
    assert!(
        matches!(
            added,
            Err(rekindle_transport::operations::friend::FriendError::NotReady(_))
        ),
        "{added:?}"
    );
    let o = hex::encode(owner);
    let stored = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::friends::load_all(conn, &o))
        .await
        .unwrap();
    assert!(stored.is_empty(), "no friend stored");
}
