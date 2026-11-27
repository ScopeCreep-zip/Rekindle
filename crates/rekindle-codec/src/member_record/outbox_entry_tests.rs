use super::*;
use crate::community::channel_record::ChannelDelete;

fn entry(prev: Option<[u8; 32]>) -> OutboxEntry {
    OutboxEntry {
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
