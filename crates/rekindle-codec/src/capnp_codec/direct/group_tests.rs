//! Stream-record and group-DM bodies hold their shapes on decode (split
//! from `tests.rs` for the file-size rule).

use super::tests::{dht_key, roster, s};
use super::*;

/// A StreamRecord whose key is empty, too long or not a record key is
/// malformed.
#[test]
fn a_stream_record_without_a_record_key_is_refused() {
    for record_key in [String::new(), "A".repeat(257), s("not-a-key")] {
        let bytes = DirectBody::StreamRecord(StreamRecord {
            record_key,
            group_id: None,
        })
        .encode();
        assert!(matches!(
            DirectBody::decode(DmTypeId::StreamRecord, &bytes),
            Err(CodecError::Deserialization(_))
        ));
    }
}

/// A roster outside 3-8 members, an empty name or content, and a group id
/// of the wrong length are refused; the signing bytes cover the roster and
/// not the key; no key is printed.
#[test]
fn group_bodies_hold_their_shapes() {
    let invite = GroupDmInvite {
        group_id: [1; 16],
        name: s("g"),
        roster: roster(),
        creator_sender_key: [0xAB; 32],
        epoch: 0,
        signature: [2; 64],
        creator_chain_key: dht_key(),
    };
    for roster in [
        roster()[..2].to_vec(),
        roster().iter().cycle().take(9).cloned().collect(),
    ] {
        let short = GroupDmInvite {
            roster,
            ..invite.clone()
        };
        let bytes = DirectBody::GroupDmInvite(short).encode();
        assert!(DirectBody::decode(DmTypeId::GroupDmInvite, &bytes).is_err());
    }
    let unnamed = DirectBody::GroupDmInvite(GroupDmInvite {
        name: String::new(),
        ..invite.clone()
    });
    assert!(DirectBody::decode(DmTypeId::GroupDmInvite, &unnamed.encode()).is_err());
    let rekeyed = GroupDmInvite {
        creator_sender_key: [0xCD; 32],
        signature: [3; 64],
        ..invite.clone()
    };
    assert_eq!(invite.signing_bytes(), rekeyed.signing_bytes());
    let mut fewer = invite.clone();
    fewer.roster[2].identity = "d".repeat(64);
    assert_ne!(invite.signing_bytes(), fewer.signing_bytes());
    assert!(!format!("{invite:?}").contains("171, 171"));
    let accept = GroupDmAccept {
        group_id: [1; 16],
        epoch: 0,
        sender_key: [0xAB; 32],
        chain_record_key: dht_key(),
    };
    assert!(!format!("{accept:?}").contains("171"));
    let mut rechained = invite.clone();
    rechained.creator_chain_key.push('x');
    assert_ne!(invite.signing_bytes(), rechained.signing_bytes());
    let empty = DirectBody::GroupContent(GroupContent {
        group_id: [1; 16],
        epoch: 0,
        sealed: Vec::new(),
    });
    assert!(DirectBody::decode(DmTypeId::GroupContent, &empty.encode()).is_err());
    let bad_id = encode_root::<crate::direct_capnp::group_dm_decline::Owned>(|mut b| {
        b.set_group_id([1; 15].as_slice());
    });
    assert!(DirectBody::decode(DmTypeId::GroupDmDecline, &bad_id).is_err());
    let bad_stream = encode_root::<crate::direct_capnp::stream_record::Owned>(|mut b| {
        b.set_record_key(dht_key());
        b.set_group_id([1; 3].as_slice());
    });
    assert!(DirectBody::decode(DmTypeId::StreamRecord, &bad_stream).is_err());
}
