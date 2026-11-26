//! A PreKeyInit carrying a `FriendAccept` (with the accepter's presence key
//! and chain record, decision 67 item 2) fits Veilid's message cap with the
//! `'D'` tag at its worst case.

use super::tests::{datagram_len, dht_key, pre_key_init, ROUTE_BLOB_LEN};
use super::*;
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

#[test]
fn a_pre_key_init_carrying_friend_accept_fits() {
    let frame = pre_key_init(&DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: dht_key(),
        route_blob: vec![0xAB; ROUTE_BLOB_LEN],
        mailbox_dht_key: dht_key(),
        friends_key: FriendsKey {
            epoch: 1,
            key: [7; 32],
        },
        chain_record_key: dht_key(),
    }));
    assert_eq!(DmFrame::decode(&frame.encode()).as_ref(), Ok(&frame));
    let len = datagram_len(&frame);
    assert!(
        len <= MAX_APP_MESSAGE_LEN,
        "PreKeyInit(FriendAccept) {len} B"
    );
}
