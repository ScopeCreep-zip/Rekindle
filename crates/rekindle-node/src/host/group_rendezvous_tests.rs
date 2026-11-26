//! A group rendezvous entry's sender against the creator's signed plan
//! (plan decision 67 item 3): an entry in row `i` is applied only when it
//! is from the member the plan gives slot `i`. A member's genuine first
//! contact, written to another member's row (a departed member's writer
//! reused, or a newcomer's), is refused and never applied.

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupId};
use rekindle_dm::GroupDmDeps;
use rekindle_transport::operations::dm::{frame, wire};
use rekindle_transport::operations::group_dm::rendezvous::landed;
use rekindle_types::dht_layout::group_rendezvous as layout;

use super::adapters::friends::HostFriends;
use super::adapters::group_dm::HostGroupDm;
use super::direct_tests::ann;
use super::group_admission_tests::{accept_of, member};
use super::prekey_tests::own_bundle;
use super::Host;

/// `from`'s wired PreKeyInit to Ann from her prekeys, carrying `first`.
async fn init_from(host: &Host, owner: [u8; 32], from: &Host, first: &DirectBody) -> Vec<u8> {
    let bundle = own_bundle(host, owner).await;
    let sent = frame::initiate_frame(
        &HostFriends::of(&from.ctx),
        &hex::encode(owner),
        bundle,
        first,
        frame::Position::default(),
    )
    .await
    .unwrap();
    wire(&sent).unwrap()
}

async fn key_of(host: &Host, gid: &GroupId, peer: [u8; 32]) -> Option<[u8; 32]> {
    let deps = HostGroupDm::open(&HostFriends::of(&host.ctx)).unwrap();
    deps.latest_sender_key(gid, &hex::encode(peer))
        .await
        .unwrap()
        .map(|k| *k.as_bytes())
}

#[tokio::test]
async fn an_entry_from_another_member_than_the_rows_is_refused() {
    let (dir, b_dir, c_dir) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let (host, owner) = ann(&dir).await;
    let (bob_host, bob) = ann(&b_dir).await;
    let (carol_host, carol) = ann(&c_dir).await;
    let friends = HostFriends::of(&host.ctx);
    let deps = HostGroupDm::open(&friends).unwrap();
    let gid = rekindle_dm::create(
        &deps,
        [7; 16],
        "raid night",
        member(owner, 'a'),
        vec![member(bob, 'b'), member(carol, 'c')],
        &format!("VLD0:{}", "Q".repeat(43)),
        &crate::host::group_dm_tests::rendezvous_mint(),
    )
    .await
    .unwrap()
    .group
    .group_id;
    let place = deps.rendezvous(&gid).await.unwrap().expect("our place");
    let slot_of = |peer: [u8; 32]| {
        let id = hex::encode(peer);
        place
            .slots
            .iter()
            .find(|(m, _)| *m == id)
            .map(|(_, s)| *s)
            .unwrap()
    };
    let gid_hex = hex::encode(gid);
    let entry = init_from(&host, owner, &carol_host, &accept_of(gid)).await;

    // Carol's genuine accept, in Bob's row of our column: refused.
    let bobs_row = layout::subkey(slot_of(bob), place.slot).unwrap();
    landed(&friends, &gid_hex, bobs_row, &entry).await;
    assert_eq!(
        key_of(&host, &gid, carol).await,
        None,
        "applied from Bob's row"
    );

    // Bob's own accept, in his row: applied.
    let bobs = init_from(&host, owner, &bob_host, &accept_of(gid)).await;
    landed(&friends, &gid_hex, bobs_row, &bobs).await;
    assert_eq!(key_of(&host, &gid, bob).await, Some([5; 32]));
}
