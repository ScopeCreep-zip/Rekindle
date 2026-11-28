use super::build_poll_states;
use crate::deps::OutboxItem;
use rekindle_codec::community::channel_record::{
    ChannelPollClose, ChannelPollCounted, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
};
use rekindle_types::id::PseudonymKey;

const NONCE: [u8; 16] = [7; 16];

/// Member 1's poll.
fn poll() -> [u8; 16] {
    crate::polls::poll_id_of(&PseudonymKey([1; 32]), &NONCE)
}

fn create(author: u8, seq: u64, at: u64, question: &str) -> OutboxItem {
    item(
        author,
        seq,
        at,
        ChannelRecordEntry::PollCreate(ChannelPollCreate {
            nonce: NONCE,
            message_id: "msg-1".into(),
            question: question.into(),
            answers: vec!["A".into(), "B".into()],
            multi_select: false,
            expires_at: None,
            lamport: seq,
        }),
    )
}

fn shown(author: u8) -> (String, String) {
    ("msg-1".into(), hex::encode([author; 32]))
}

fn item(author: u8, seq: u64, at: u64, entry: ChannelRecordEntry) -> OutboxItem {
    OutboxItem {
        admission: None,
        author: PseudonymKey([author; 32]),
        stream_seq: seq,
        committed_at: at,
        entry,
    }
}

fn vote(voter: u8, seq: u64, at: u64, answer: u8) -> OutboxItem {
    item(
        voter,
        seq,
        at,
        ChannelRecordEntry::PollVote(ChannelPollVote {
            poll_id: poll(),
            selected_answers: vec![answer],
            lamport: seq,
        }),
    )
}

fn counted(voter: u8, stream_seq: u64) -> ChannelPollCounted {
    ChannelPollCounted {
        voter: hex::encode([voter; 32]),
        stream_seq,
    }
}

/// The author's close counts exactly the votes it names; a vote it does
/// not name does not count, however its voter dated it (members 4 and
/// 5, and member 2's later vote, backdated before the close), and a
/// close by another member closes nothing.
#[test]
fn the_latest_vote_before_the_authors_close_counts() {
    let entries = vec![
        vote(2, 2, 300, 1),
        create(1, 1, 100, "Pick one"),
        vote(2, 1, 200, 0),
        item(
            3,
            1,
            350,
            ChannelRecordEntry::PollClose(ChannelPollClose {
                poll_id: poll(),
                lamport: 1,
                close_id: [1; 16],
                part: 0,
                parts: 1,
                counted: Vec::new(),
            }),
        ),
        vote(3, 2, 360, 0),
        item(
            1,
            2,
            400,
            ChannelRecordEntry::PollClose(ChannelPollClose {
                poll_id: poll(),
                lamport: 2,
                close_id: [2; 16],
                part: 0,
                parts: 1,
                counted: vec![counted(2, 2), counted(3, 2)],
            }),
        ),
        vote(4, 1, 500, 0),
        vote(5, 1, 150, 0),
        vote(2, 3, 390, 0),
    ];
    let voter = hex::encode([2u8; 32]);
    let polls = build_poll_states(&entries, &voter);
    let poll = &polls[&shown(1)];
    assert!(poll.closed);
    assert_eq!(poll.selected_answers, vec![1]);
    assert_eq!(poll.answers[1].voters, vec![voter]);
    // Only the votes the close names count.
    assert_eq!(poll.answers[0].voters, vec![hex::encode([3u8; 32])]);
}

/// A create by another member, backdated before the creator's and with
/// the creator's nonce, names another poll: the creator's poll keeps its
/// question and votes, and the other member's poll never shows on the
/// creator's message.
#[test]
fn a_backdated_create_by_another_member_takes_no_poll_over() {
    let entries = vec![
        create(1, 1, 100, "Pick one"),
        create(9, 1, 1, "Hijacked"),
        vote(2, 1, 200, 1),
    ];
    let tallies = super::poll_tallies(&entries);
    assert_eq!(tallies[&poll()].question, "Pick one");
    assert_eq!(tallies[&poll()].author, hex::encode([1u8; 32]));
    let polls = build_poll_states(&entries, "");
    assert_eq!(polls[&shown(1)].question, "Pick one");
    assert_eq!(polls[&shown(1)].answers[1].vote_count, 1);
    // Member 9's own poll is keyed to member 9: it shows only on a
    // message whose sender is member 9, never on member 1's.
    assert_eq!(polls[&shown(9)].question, "Hijacked");
}
