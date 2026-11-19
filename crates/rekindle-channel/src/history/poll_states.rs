//! A channel's polls (plan E3.3): each poll's create, close and votes are
//! ops on their authors' channel outboxes, aggregated by every reader. A
//! poll belongs to the author of its first create (by commit time, ties by
//! author); only that author's later creates replace it and only its close
//! closes it. Each voter counts once, its latest vote by stream seq, and a
//! vote committed after the close is not counted (commit times are signed
//! with each entry; an author can only misdate its own ops).

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
    /// (commit time, stream seq) of the create kept.
    order: (u64, u64),
}

/// Every poll among `entries`, by poll id.
#[must_use]
pub fn poll_tallies(entries: &[OutboxItem]) -> BTreeMap<[u8; 16], PollTally> {
    let mut polls: BTreeMap<[u8; 16], PollTally> = BTreeMap::new();
    let mut creates: Vec<&OutboxItem> = (entries.iter())
        .filter(|i| matches!(i.entry, ChannelRecordEntry::PollCreate(_)))
        .collect();
    creates.sort_by_key(|i| (i.committed_at, i.author.0, i.stream_seq));
    for item in creates {
        let ChannelRecordEntry::PollCreate(create) = &item.entry else {
            continue;
        };
        let author = hex::encode(item.author.0);
        let replace = polls.get(&create.poll_id).is_none_or(|p| {
            p.author == author && p.closed_at.is_none() && item.stream_seq > p.order.1
        });
        if replace {
            polls.insert(
                create.poll_id,
                PollTally {
                    author,
                    message_id: create.message_id.clone(),
                    question: create.question.clone(),
                    answers: create.answers.clone(),
                    multi_select: create.multi_select,
                    expires_at: create.expires_at,
                    closed_at: None,
                    votes: BTreeMap::new(),
                    order: (item.committed_at, item.stream_seq),
                },
            );
        }
    }
    for item in entries {
        if let ChannelRecordEntry::PollClose(close) = &item.entry {
            if let Some(poll) = polls.get_mut(&close.poll_id) {
                if poll.author == hex::encode(item.author.0) {
                    poll.closed_at = Some(
                        poll.closed_at
                            .map_or(item.committed_at, |at| at.min(item.committed_at)),
                    );
                }
            }
        }
    }
    let mut latest: HashMap<([u8; 16], String), (u64, Vec<u8>)> = HashMap::new();
    for item in entries {
        let ChannelRecordEntry::PollVote(vote) = &item.entry else {
            continue;
        };
        let Some(poll) = polls.get(&vote.poll_id) else {
            continue;
        };
        if poll.closed_at.is_some_and(|at| item.committed_at > at) {
            continue;
        }
        let key = (vote.poll_id, hex::encode(item.author.0));
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
    for ((poll_id, voter), (_, answers)) in latest {
        if let Some(poll) = polls.get_mut(&poll_id) {
            poll.votes.insert(voter, answers);
        }
    }
    polls
}

/// Each poll among `entries` as the message it is attached to shows it
/// (`my_pseudonym`'s own answers selected); a message with two polls shows
/// the later created.
#[must_use]
pub fn build_poll_states(
    entries: &[OutboxItem],
    my_pseudonym: &str,
) -> HashMap<String, MessagePoll> {
    let mut by_message: BTreeMap<String, ((u64, u64), MessagePoll)> = BTreeMap::new();
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
        if by_message
            .get(&poll.message_id)
            .is_none_or(|(order, _)| poll.order >= *order)
        {
            by_message.insert(poll.message_id, (poll.order, shown));
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
        ChannelPollClose, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
    };
    use rekindle_types::id::PseudonymKey;

    const POLL: [u8; 16] = [7; 16];

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
                poll_id: POLL,
                selected_answers: vec![answer],
                lamport: seq,
            }),
        )
    }

    /// Each voter's latest vote by stream seq counts; a vote committed after
    /// the author's close does not, and a close by another member closes
    /// nothing.
    #[test]
    fn the_latest_vote_before_the_authors_close_counts() {
        let entries = vec![
            vote(2, 2, 300, 1),
            item(
                1,
                1,
                100,
                ChannelRecordEntry::PollCreate(ChannelPollCreate {
                    poll_id: POLL,
                    message_id: "msg-1".into(),
                    question: "Pick one".into(),
                    answers: vec!["A".into(), "B".into()],
                    multi_select: false,
                    expires_at: None,
                    lamport: 1,
                }),
            ),
            vote(2, 1, 200, 0),
            item(
                3,
                1,
                350,
                ChannelRecordEntry::PollClose(ChannelPollClose {
                    poll_id: POLL,
                    lamport: 1,
                }),
            ),
            vote(3, 2, 360, 0),
            item(
                1,
                2,
                400,
                ChannelRecordEntry::PollClose(ChannelPollClose {
                    poll_id: POLL,
                    lamport: 2,
                }),
            ),
            vote(4, 1, 500, 0),
        ];
        let voter = hex::encode([2u8; 32]);
        let polls = build_poll_states(&entries, &voter);
        let poll = &polls["msg-1"];
        assert!(poll.closed);
        assert_eq!(poll.selected_answers, vec![1]);
        assert_eq!(poll.answers[1].voters, vec![voter]);
        // Member 3's vote lands before the author's close; member 4's after.
        assert_eq!(poll.answers[0].voters, vec![hex::encode([3u8; 32])]);
    }
}
