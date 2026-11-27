use super::*;
use rekindle_codec::community::channel_record::ChannelDelete;

fn pending(stream_seq: u64, message_id: String) -> PendingEntry {
    PendingEntry {
        channel_tag: [7; 16],
        stream_seq,
        generation: 1,
        entry: ChannelRecordEntry::Delete(ChannelDelete {
            message_id,
            deleted_at: 1,
            lamport_ts: stream_seq,
        }),
    }
}

/// Each entry's `prev` is the hash of the one before it, the first
/// links to the chain's tip, and each is signed at its own chain seq.
#[test]
fn a_batch_links_each_entry_to_the_one_before() {
    let author = SigningKey::from_bytes(&[3; 32]);
    let public = author.verifying_key().to_bytes();
    let tip = ChainTip {
        seq: 4,
        bytes: b"previous entry".to_vec(),
    };
    let batch = [pending(1, "a".into()), pending(2, "b".into())];
    let sealed = seal_batch(Some(&tip), "anchor", &author, 10, &batch).unwrap();
    let at = |chain_seq| ChainPosition {
        anchor: "anchor",
        chain_seq,
    };
    let first = decode_outbox_entry(&sealed[0], at(5), &public).unwrap();
    let second = decode_outbox_entry(&sealed[1], at(6), &public).unwrap();
    assert_eq!(first.prev, Some(entry_hash(&tip.bytes)));
    assert_eq!(second.prev, Some(entry_hash(&sealed[0])));
    assert!(decode_outbox_entry(&sealed[1], at(5), &public).is_err());
    let fresh = seal_batch(None, "anchor", &author, 10, &batch[..1]).unwrap();
    assert_eq!(
        decode_outbox_entry(&fresh[0], at(1), &public).unwrap().prev,
        None
    );
}

/// A reader checks each entry's link: to the entry before it, or to
/// nothing for the chain's first; any other bytes break the chain.
#[test]
fn a_reader_finds_a_broken_link() {
    use rekindle_codec::member_record::links_to;
    let author = SigningKey::from_bytes(&[3; 32]);
    let public = author.verifying_key().to_bytes();
    let batch = [pending(1, "a".into()), pending(2, "b".into())];
    let sealed = seal_batch(None, "anchor", &author, 10, &batch).unwrap();
    let at = |chain_seq| ChainPosition {
        anchor: "anchor",
        chain_seq,
    };
    let first = decode_outbox_entry(&sealed[0], at(1), &public).unwrap();
    let second = decode_outbox_entry(&sealed[1], at(2), &public).unwrap();
    assert!(links_to(&first, None));
    assert!(links_to(&second, Some(entry_hash(&sealed[0]))));
    assert!(
        !links_to(&second, None),
        "the second is not a chain's first"
    );
    assert!(!links_to(&second, Some(entry_hash(&sealed[1]))));
    assert!(!links_to(
        &first,
        Some(entry_hash(b"an entry before the first"))
    ));
}

/// An entry no page holds alone is refused with `EntryTooLarge`.
#[test]
fn an_oversized_entry_is_refused() {
    let author = SigningKey::from_bytes(&[3; 32]);
    let huge = pending(1, "x".repeat(40_000));
    let refused = seal_batch(None, "anchor", &author, 10, &[huge]).unwrap_err();
    assert!(
        matches!(refused, ChannelError::EntryTooLarge { .. }),
        "{refused:?}"
    );
    assert!(seal_batch(
        None,
        "anchor",
        &author,
        10,
        &[pending(1, "x".repeat(20_000))]
    )
    .is_ok());
}
