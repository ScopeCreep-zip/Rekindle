//! A channel's polls (plan E3.3): each poll's create, close and votes are
//! ops on their authors' channel outboxes, aggregated by every reader. A
//! poll's id is derived from its creator's pseudonym and the create's nonce
//! ([`crate::poll_id_of`]), so only the creator's creates name it (rule E1:
//! no commit time, which the author signs and could backdate, decides
//! ownership); its latest create by stream seq before the close is kept, and
//! only the creator's close closes it. A poll shows on its message only when
//! the message is the creator's. Before the close each voter counts once, its latest vote by
//! stream seq. The close names the vote it counted for each voter (voter,
//! stream seq), and from then on readers count exactly those votes: no
//! cross-author clock exists (rule E1), so a vote's place against the close
//! is the closer's view of the voter's stream, as a departure head names a
//! departed author's chain position, never the voter's own signed commit
//! time, which the voter could backdate.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_types::dto::channel_message::{MessagePoll, MessagePollAnswer};

use crate::deps::OutboxItem;

/// One poll as its entries leave it.
#[derive(Debug, Clone)]
pub struct PollTally {
    /// The author's pseudonym hex.
    pub author: String,
    pub message_id: String,
    pub question: String,
    pub answers: Vec<String>,
    pub multi_select: bool,
    pub expires_at: Option<u64>,
    /// The close's commit time, ms.
    pub closed_at: Option<u64>,
    /// Each voter's counted answers.
    pub votes: BTreeMap<String, Vec<u8>>,
    /// Each counted voter's stream seq of the vote counted.
    pub vote_seqs: BTreeMap<String, u64>,
    /// The creator's stream seq of the create kept.
    order: u64,
}

/// Every poll among `entries`, by poll id.
#[must_use]
pub fn poll_tallies(entries: &[OutboxItem]) -> BTreeMap<[u8; 16], PollTally> {
    let mut polls: BTreeMap<[u8; 16], PollTally> = BTreeMap::new();
    let mut creates: Vec<&OutboxItem> = (entries.iter())
        .filter(|i| matches!(i.entry, ChannelRecordEntry::PollCreate(_)))
        .collect();
    creates.sort_by_key(|i| (i.author.0, i.stream_seq));
    for item in creates {
        let ChannelRecordEntry::PollCreate(create) = &item.entry else {
            continue;
        };
        let poll_id = crate::polls::poll_id_of(&item.author, &create.nonce);
        // The id binds the creator, so only its own later create replaces.
        if polls
            .get(&poll_id)
            .is_none_or(|p| item.stream_seq > p.order)
        {
            polls.insert(
                poll_id,
                PollTally {
                    author: hex::encode(item.author.0),
                    message_id: create.message_id.clone(),
                    question: create.question.clone(),
                    answers: create.answers.clone(),
                    multi_select: create.multi_select,
                    expires_at: create.expires_at,
                    closed_at: None,
                    votes: BTreeMap::new(),
                    vote_seqs: BTreeMap::new(),
                    order: item.stream_seq,
                },
            );
        }
    }
    // The author's first close by stream seq closes the poll.
    let mut closes: HashMap<[u8; 16], (u64, u64, HashMap<String, u64>)> = HashMap::new();
    for item in entries {
        let ChannelRecordEntry::PollClose(close) = &item.entry else {
            continue;
        };
        let Some(poll) = polls.get(&close.poll_id) else {
            continue;
        };
        if poll.author != hex::encode(item.author.0)
            || closes
                .get(&close.poll_id)
                .is_some_and(|(seq, _, _)| *seq <= item.stream_seq)
        {
            continue;
        }
        let counted = (close.counted.iter())
            .map(|c| (c.voter.clone(), c.stream_seq))
            .collect();
        closes.insert(close.poll_id, (item.stream_seq, item.committed_at, counted));
    }
    let mut latest: HashMap<([u8; 16], String), (u64, Vec<u8>)> = HashMap::new();
    for item in entries {
        let ChannelRecordEntry::PollVote(vote) = &item.entry else {
            continue;
        };
        let Some(poll) = polls.get(&vote.poll_id) else {
            continue;
        };
        let voter = hex::encode(item.author.0);
        // Once closed, only the vote the close names for this voter counts.
        if let Some((_, _, counted)) = closes.get(&vote.poll_id) {
            if counted.get(&voter) != Some(&item.stream_seq) {
                continue;
            }
        }
        let key = (vote.poll_id, voter);
        if latest
            .get(&key)
            .is_none_or(|(seq, _)| item.stream_seq > *seq)
        {
            let answers = sanitize(
                vote.selected_answers.clone(),
                poll.answers.len(),
                poll.multi_select,
            );
            latest.insert(key, (item.stream_seq, answers));
        }
    }
    for (poll_id, (_, closed_at, _)) in closes {
        if let Some(poll) = polls.get_mut(&poll_id) {
            poll.closed_at = Some(closed_at);
        }
    }
    for ((poll_id, voter), (seq, answers)) in latest {
        if let Some(poll) = polls.get_mut(&poll_id) {
            poll.vote_seqs.insert(voter.clone(), seq);
            poll.votes.insert(voter, answers);
        }
    }
    polls
}

/// A message's poll, keyed by (message id, the creator's pseudonym hex): a
/// poll shows only on its creator's own message.
pub type PollsByMessage = HashMap<(String, String), MessagePoll>;

/// Each poll among `entries` as the message it is attached to shows it
/// (`my_pseudonym`'s own answers selected), keyed by (message id, creator);
/// a creator's two polls on one message show the later by its stream seq.
#[must_use]
pub fn build_poll_states(entries: &[OutboxItem], my_pseudonym: &str) -> PollsByMessage {
    let mut by_message: BTreeMap<(String, String), (u64, MessagePoll)> = BTreeMap::new();
    for (poll_id, poll) in poll_tallies(entries) {
        let mut voters: Vec<BTreeSet<String>> = vec![BTreeSet::new(); poll.answers.len()];
        for (voter, answers) in &poll.votes {
            for answer in answers {
                if let Some(set) = voters.get_mut(usize::from(*answer)) {
                    set.insert(voter.clone());
                }
            }
        }
        let answers = (poll.answers.iter().enumerate())
            .map(|(idx, text)| {
                let voters: Vec<String> = voters
                    .get(idx)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                MessagePollAnswer {
                    index: u8::try_from(idx).unwrap_or(u8::MAX),
                    text: text.clone(),
                    vote_count: u32::try_from(voters.len()).unwrap_or(u32::MAX),
                    voters,
                }
            })
            .collect();
        let shown = MessagePoll {
            poll_id: hex::encode(poll_id),
            question: poll.question,
            answers,
            multi_select: poll.multi_select,
            expires_at: poll.expires_at,
            closed: poll.closed_at.is_some(),
            selected_answers: poll.votes.get(my_pseudonym).cloned().unwrap_or_default(),
        };
        let key = (poll.message_id, poll.author);
        if by_message
            .get(&key)
            .is_none_or(|(order, _)| poll.order >= *order)
        {
            by_message.insert(key, (poll.order, shown));
        }
    }
    by_message
        .into_iter()
        .map(|(id, (_, poll))| (id, poll))
        .collect()
}

fn sanitize(selected: Vec<u8>, answer_count: usize, multi_select: bool) -> Vec<u8> {
    let mut answers: Vec<u8> = (selected.into_iter())
        .filter(|answer| usize::from(*answer) < answer_count)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    answers.sort_unstable();
    if !multi_select {
        answers.truncate(1);
    }
    answers
}

#[cfg(test)]
mod tests {
    use super::build_poll_states;
    use crate::deps::OutboxItem;
    use rekindle_codec::community::channel_record::{
        ChannelPollClose, ChannelPollCounted, ChannelPollCreate, ChannelPollVote,
        ChannelRecordEntry,
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
}
