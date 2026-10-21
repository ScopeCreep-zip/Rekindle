//! Group members who are not friends (plan E2.6 G2d): a roster member's
//! first group frame opens the pair's session from our prekeys and is
//! admitted; the same init from someone on no roster, or carrying a
//! friendship kind, is refused before any prekey is looked up; and a
//! leave ends the session with a member who is not a friend while a
//! friend's 1:1 session stays.

use rekindle_codec::capnp_codec::direct::{DirectBody, FriendAccept, GroupDmAccept, GroupId};
use rekindle_db::ratchet::PeerAddress;
use rekindle_dm::GroupDmDeps;
use rekindle_transport::operations::direct::receive;
use rekindle_transport::operations::dm::{frame, wire};
use rekindle_types::group_dm::GroupDmRosterMember;

use super::adapters::friends::HostFriends;
use super::adapters::group_dm::HostGroupDm;
use super::direct_tests::{ann, befriend, text, unapplied};
use super::prekey_tests::own_bundle;
use super::Host;

fn member(identity: [u8; 32], n: char) -> GroupDmRosterMember {
    GroupDmRosterMember {
        identity: hex::encode(identity),
        profile_record_key: format!("VLD0:{}", n.to_string().repeat(43)),
    }
}

/// Ann's group of herself and `others`; its id.
async fn group_of(host: &Host, owner: [u8; 32], others: &[[u8; 32]]) -> GroupId {
    let deps = HostGroupDm::open(&HostFriends::of(&host.ctx)).unwrap();
    let others = others.iter().map(|k| member(*k, 'b')).collect();
    rekindle_dm::create(
        &deps,
        [7; 16],
        "raid night",
        member(owner, 'a'),
        others,
        &format!("VLD0:{}", "Q".repeat(43)),
    )
    .await
    .unwrap()
    .group
    .group_id
}

fn accept_of(group_id: GroupId) -> DirectBody {
    DirectBody::GroupDmAccept(GroupDmAccept {
        group_id,
        epoch: 0,
        sender_key: [5; 32],
        chain_record_key: format!("VLD0:{}", "R".repeat(43)),
    })
}

/// `from`'s PreKeyInit to Ann from her prekeys, carrying `first`, received.
async fn init_from(host: &Host, owner: [u8; 32], from: &Host, first: &DirectBody) {
    let bundle = own_bundle(host, owner).await;
    let sender = HostFriends::of(&from.ctx);
    let sent = frame::initiate_frame(&sender, &hex::encode(owner), bundle, first, 0)
        .await
        .unwrap();
    receive(&HostFriends::of(&host.ctx), &wire(&sent).unwrap()).await;
}

async fn established(host: &Host, owner: [u8; 32], peer: [u8; 32]) -> bool {
    let db = host.ctx.db.current().unwrap();
    rekindle_db::ratchet::is_established(&db, &owner, &PeerAddress::sole_device(peer))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_members_first_group_frame_opens_the_session_and_a_strangers_does_not() {
    let (dir, b_dir, s_dir) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let (host, owner) = ann(&dir).await;
    let (bob_host, bob) = ann(&b_dir).await;
    let (stranger_host, stranger) = ann(&s_dir).await;
    let gid = group_of(&host, owner, &[bob, [9; 32]]).await;

    // Someone on no roster, with the identical init: refused.
    init_from(&host, owner, &stranger_host, &accept_of(gid)).await;
    assert!(!established(&host, owner, stranger).await);

    // Bob, a member who is not a friend, with a friendship kind: refused.
    let friend_accept = DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: String::new(),
        route_blob: Vec::new(),
        mailbox_dht_key: String::new(),
    });
    init_from(&host, owner, &bob_host, &friend_accept).await;
    assert!(!established(&host, owner, bob).await);
    assert_eq!(unapplied(&host, owner).await, 0);

    // Bob's group accept opens the pair's session and is applied.
    init_from(&host, owner, &bob_host, &accept_of(gid)).await;
    assert!(established(&host, owner, bob).await);
    let deps = HostGroupDm::open(&HostFriends::of(&host.ctx)).unwrap();
    let key = deps
        .latest_sender_key(&gid, &hex::encode(bob))
        .await
        .unwrap();
    assert_eq!(key.map(|k| *k.as_bytes()), Some([5; 32]));
}

#[tokio::test]
async fn leaving_ends_a_members_session_and_keeps_a_friends() {
    let (dir, b_dir, c_dir) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let (host, owner) = ann(&dir).await;
    let (bob_host, bob) = ann(&b_dir).await;
    let (carol_host, carol) = ann(&c_dir).await;
    befriend(&host, owner, carol).await;
    let gid = group_of(&host, owner, &[bob, carol]).await;

    init_from(&host, owner, &bob_host, &accept_of(gid)).await;
    init_from(&host, owner, &carol_host, &text(1)).await;
    assert!(established(&host, owner, bob).await);
    assert!(established(&host, owner, carol).await);

    // The leave rows are committed (or refused offline); the sessions end
    // either way.
    let friends = HostFriends::of(&host.ctx);
    let _ = rekindle_transport::operations::group_dm::leave(&friends, &hex::encode(gid)).await;
    assert!(
        !established(&host, owner, bob).await,
        "a member's session ends"
    );
    assert!(established(&host, owner, carol).await, "a friend's stays");
}
