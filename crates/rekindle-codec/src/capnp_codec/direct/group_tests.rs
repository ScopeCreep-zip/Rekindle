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
            DirectBody::decode(DmTypeId::StreamRecord, &bytes.unwrap()),
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
        rendezvous_key: dht_key(),
        slot: 1,
        slot_writer: s("VLD0:writer"),
        slots: vec![0, 1, 2],
    };
    for roster in [
        roster()[..2].to_vec(),
        roster().iter().cycle().take(9).cloned().collect(),
    ] {
        let short = GroupDmInvite {
            slots: (0..u8::try_from(roster.len()).unwrap_or(9)).collect(),
            roster,
            ..invite.clone()
        };
        let bytes = DirectBody::GroupDmInvite(short).encode();
        assert!(DirectBody::decode(DmTypeId::GroupDmInvite, &bytes.unwrap()).is_err());
    }
    let unnamed = DirectBody::GroupDmInvite(GroupDmInvite {
        name: String::new(),
        ..invite.clone()
    });
    assert!(DirectBody::decode(DmTypeId::GroupDmInvite, &unnamed.encode().unwrap()).is_err());
    let rekeyed = GroupDmInvite {
        creator_sender_key: [0xCD; 32],
        signature: [3; 64],
        ..invite.clone()
    };
    assert_eq!(invite.signing_bytes(), rekeyed.signing_bytes());
    // Decision 67 item 3: the rendezvous record and the slot plan are
    // signed; the recipient's own slot and writer are not.
    let resent = GroupDmInvite {
        slot: 2,
        slot_writer: s("VLD0:other-writer"),
        ..invite.clone()
    };
    assert_eq!(invite.signing_bytes(), resent.signing_bytes());
    let moved = GroupDmInvite {
        rendezvous_key: s("VLD0:elsewhere"),
        ..invite.clone()
    };
    assert_ne!(invite.signing_bytes(), moved.signing_bytes());
    let replanned = GroupDmInvite {
        slots: vec![0, 2, 1],
        ..invite.clone()
    };
    assert_ne!(invite.signing_bytes(), replanned.signing_bytes());
    assert!(!format!("{invite:?}").contains("VLD0:writer"));
    let decoded = DirectBody::decode(
        DmTypeId::GroupDmInvite,
        &DirectBody::GroupDmInvite(invite.clone()).encode().unwrap(),
    );
    assert!(matches!(decoded, Ok(DirectBody::GroupDmInvite(ref back)) if *back == invite));
    for (slot, slots) in [
        (8, vec![0, 1, 2]),
        (1, vec![0, 1]),
        (1, vec![0, 1, 1]),
        (1, vec![0, 1, 9]),
    ] {
        let bad = DirectBody::GroupDmInvite(GroupDmInvite {
            slot,
            slots,
            ..invite.clone()
        });
        assert!(DirectBody::decode(DmTypeId::GroupDmInvite, &bad.encode().unwrap()).is_err());
    }
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
    assert!(DirectBody::decode(DmTypeId::GroupContent, &empty.encode().unwrap()).is_err());
    let bad_id = encode_root::<crate::direct_capnp::group_dm_decline::Owned>(|mut b| {
        b.set_group_id([1; 15].as_slice());
        Ok(())
    });
    assert!(DirectBody::decode(DmTypeId::GroupDmDecline, &bad_id.unwrap()).is_err());
    let bad_stream = encode_root::<crate::direct_capnp::stream_record::Owned>(|mut b| {
        b.set_record_key(dht_key());
        b.set_group_id([1; 3].as_slice());
        Ok(())
    });
    assert!(DirectBody::decode(DmTypeId::StreamRecord, &bad_stream.unwrap()).is_err());
}
