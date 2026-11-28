use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::{decode_outbox_entry, ChainPosition, OutboxEntry};
use crate::capnp_codec::{bounds, pack, unpack_bounded};
use crate::community::channel_record::{ChannelDelete, ChannelMessage, ChannelRecordEntry};
use crate::member_record_capnp::outbox_entry;

fn entry(prev: Option<[u8; 32]>) -> OutboxEntry {
    OutboxEntry {
        governance_cut: Vec::new(),
        prev,
        channel_tag: [5; 16],
        stream_seq: 9,
        generation: 2,
        committed_at: 1_700_000_000_000,
        entry: ChannelRecordEntry::Delete(ChannelDelete {
            message_id: "msg_1".into(),
            deleted_at: 1,
            lamport_ts: 3,
        }),
    }
}

const AT: ChainPosition<'static> = ChainPosition {
    anchor: "VLD0:anchor",
    chain_seq: 4,
};

#[test]
fn an_entry_round_trips_under_its_author_and_position() {
    let author = SigningKey::from_bytes(&[3; 32]);
    let public = author.verifying_key().to_bytes();
    let bytes = entry(Some([1; 32])).encode(AT, &author).unwrap();
    let back = decode_outbox_entry(&bytes, AT, &public).unwrap();
    assert_eq!(back.prev, Some([1; 32]));
    assert_eq!((back.stream_seq, back.generation), (9, 2));
    assert!(matches!(back.entry, ChannelRecordEntry::Delete(_)));
    let first = entry(None).encode(AT, &author).unwrap();
    assert_eq!(decode_outbox_entry(&first, AT, &public).unwrap().prev, None);
}

/// Another chain seq, anchor or author does not verify.
#[test]
fn an_entry_is_bound_to_its_chain_position_and_author() {
    let author = SigningKey::from_bytes(&[3; 32]);
    let public = author.verifying_key().to_bytes();
    let bytes = entry(None).encode(AT, &author).unwrap();
    let moved = ChainPosition { chain_seq: 5, ..AT };
    assert!(decode_outbox_entry(&bytes, moved, &public).is_err());
    let other = ChainPosition {
        anchor: "VLD0:other",
        ..AT
    };
    assert!(decode_outbox_entry(&bytes, other, &public).is_err());
    let stranger = SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes();
    assert!(decode_outbox_entry(&bytes, AT, &stranger).is_err());
}

#[test]
fn changing_a_governance_head_breaks_the_outbox_signature() {
    let author = SigningKey::from_bytes(&[3; 32]);
    let public = author.verifying_key().to_bytes();
    let mut original = entry(None);
    original.governance_cut.push(OpRef {
        op_hash: OpHash([12; 32]),
        author: PseudonymKey(public),
        author_record: "VLD0:fixture".into(),
        author_seq: 1,
        device_key: public,
    });
    assert!(original.encode(AT, &author).is_err(), "a delete has no cut");
    original.entry = ChannelRecordEntry::Message(ChannelMessage {
        sequence: 9,
        sender_pseudonym: hex::encode(public),
        ciphertext: vec![1, 2, 3],
        mek_generation: 2,
        timestamp: original.committed_at,
        reply_to: None,
        revives: None,
        lamport_ts: 9,
        message_id: Some("msg_1".into()),
        attachment: None,
        flags: 0,
        mentioned_pseudonyms: Vec::new(),
        mentioned_roles: Vec::new(),
    });
    let signed = original.encode(AT, &author).unwrap();
    let decoded = decode_outbox_entry(&signed, AT, &public).unwrap();
    assert_eq!(decoded.governance_cut, original.governance_cut);

    let packed = unpack_bounded(&signed, bounds::OUTBOX_ENTRY_BOUND).unwrap();
    let root = packed.get_root::<outbox_entry::Reader<'_>>().unwrap();
    let mut altered = capnp::message::Builder::new_default();
    altered.set_root(root).unwrap();
    altered.get_root::<outbox_entry::Builder<'_>>().unwrap()
        .get_governance_cut().unwrap().get(0).set_author_seq(2);
    let altered = pack(&altered).unwrap();
    assert!(decode_outbox_entry(&altered, AT, &public).is_err());
}
