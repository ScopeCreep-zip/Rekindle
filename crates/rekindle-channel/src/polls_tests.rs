use super::*;

#[test]
fn validate_poll_create_rejects_empty_question() {
    let err = validate_poll_create("   ", &["a".into(), "b".into()]).unwrap_err();
    assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("empty")));
}

#[test]
fn validate_poll_create_rejects_too_few_answers() {
    let err = validate_poll_create("q", &["only".into()]).unwrap_err();
    assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("2 and 10")));
}

#[test]
fn validate_poll_create_rejects_too_many_answers() {
    let answers: Vec<String> = (0..11).map(|i| format!("a{i}")).collect();
    let err = validate_poll_create("q", &answers).unwrap_err();
    assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("2 and 10")));
}

#[test]
fn validate_poll_create_rejects_empty_answer() {
    let err = validate_poll_create("q", &["a".into(), "  ".into()]).unwrap_err();
    assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("empty")));
}

#[test]
fn validate_poll_create_accepts_valid_input() {
    let result = validate_poll_create("Best color?", &["red".into(), "blue".into()]);
    assert!(result.is_ok());
}

#[test]
fn parse_poll_id_rejects_short_hex() {
    let err = parse_poll_id("abcd").unwrap_err();
    assert!(matches!(err, ChannelError::Adapter(_)));
}

#[test]
fn parse_poll_id_accepts_32_hex_chars() {
    let id = parse_poll_id("00112233445566778899aabbccddeeff").unwrap();
    assert_eq!(
        id,
        [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff
        ]
    );
}

#[test]
fn dedupe_selected_answers_sorts_and_dedups() {
    assert_eq!(dedupe_selected_answers(vec![2, 0, 1, 0, 2]), vec![0, 1, 2]);
}

#[test]
fn a_poll_id_is_bound_to_its_creator() {
    let nonce = [9u8; 16];
    let ann = poll_id_of(&PseudonymKey([1; 32]), &nonce);
    assert_eq!(ann, poll_id_of(&PseudonymKey([1; 32]), &nonce));
    assert_ne!(ann, poll_id_of(&PseudonymKey([2; 32]), &nonce));
}

/// A close over 1,000 voters (each at the largest seq) is parts that
/// each fit one entry, and a reader that has read every part counts
/// every voter; one part short, the poll is still open.
#[test]
fn a_close_past_one_page_is_parts_that_each_fit() {
    use crate::deps::OutboxItem;
    use crate::outbox_chain::{seal_batch, PendingEntry};
    use rekindle_codec::community::channel_record::{
        ChannelPollClose, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
    };
    let creator = PseudonymKey([1; 32]);
    let nonce = [5u8; 16];
    let poll_id = poll_id_of(&creator, &nonce);
    let voter = |i: u16| {
        let mut key = [0u8; 32];
        key[..2].copy_from_slice(&i.to_be_bytes());
        key[31] = 0xee;
        PseudonymKey(key)
    };
    let counted: Vec<(String, u64)> = (0..1000)
        .map(|i| (hex::encode(voter(i).0), u64::MAX))
        .collect();
    let parts = poll_close_parts(poll_id, &counted).unwrap();
    assert!(parts.len() > 1);
    let author = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[3; 32]);
    let mut entries = vec![OutboxItem {
        author: creator.clone(),
        stream_seq: 1,
        committed_at: 1,
        entry: ChannelRecordEntry::PollCreate(ChannelPollCreate {
            nonce,
            message_id: "m".into(),
            question: "q".into(),
            answers: vec!["a".into(), "b".into()],
            multi_select: false,
            expires_at: None,
            lamport: 1,
        }),
    }];
    entries.extend((0..1000).map(|i| OutboxItem {
        author: voter(i),
        stream_seq: u64::MAX,
        committed_at: 2,
        entry: ChannelRecordEntry::PollVote(ChannelPollVote {
            poll_id,
            selected_answers: vec![0],
            lamport: 1,
        }),
    }));
    for (seq, part) in (2u64..).zip(parts) {
        let PostEntry::PollClose {
            close_id,
            part,
            parts,
            counted,
            ..
        } = part
        else {
            panic!("a close part");
        };
        let entry = ChannelRecordEntry::PollClose(ChannelPollClose {
            poll_id,
            lamport: seq,
            close_id,
            part,
            parts,
            counted: counted_entries(&counted),
        });
        let pending = PendingEntry {
            channel_tag: [7; 16],
            stream_seq: u64::MAX,
            generation: u64::MAX,
            entry: entry.clone(),
        };
        seal_batch(None, &"a".repeat(64), &author, u64::MAX, &[pending])
            .expect("each part fits one entry");
        entries.push(OutboxItem {
            author: creator.clone(),
            stream_seq: seq,
            committed_at: 3,
            entry,
        });
    }
    let short = &entries[..entries.len() - 1];
    assert!(crate::history::poll_tallies(short)[&poll_id]
        .closed_at
        .is_none());
    let poll = &crate::history::poll_tallies(&entries)[&poll_id];
    assert!(poll.closed_at.is_some());
    assert_eq!(poll.votes.len(), 1000);
}
