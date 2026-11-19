//! A channel record's reactions and polls as messages carry them (plan E3.1
//! E2c, from the desktop's `channel_materialize`): each member's latest
//! reaction per expression, and each poll with its latest vote per member
//! before it closed. Pure: the entries come from the host's record read.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::BuildHasher;

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_types::dto::channel_message::{MessagePoll, MessagePollAnswer, ReactionGroup};

use crate::deps::{ChannelEntryItem, OutboxItem};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EntryOrder {
    lamport: u64,
    subkey_index: u32,
}

#[derive(Clone)]
struct PollCreateState {
    order: EntryOrder,
    author_subkey: u32,
    message_id: String,
    poll_id: [u8; 16],
    question: String,
    answers: Vec<String>,
    multi_select: bool,
    expires_at: Option<u64>,
    closed_at: Option<EntryOrder>,
}

/// The most distinct reactions one message carries (rule R2; Discord's
/// limit of 20 unique reactions per message): further emojis are not
/// counted.
pub const MAX_REACTIONS_PER_MESSAGE: usize = 20;

/// The longest reaction key, bytes (rule R2): a `custom:` expression id
/// is 39, the longest emoji ZWJ sequences stay under it.
pub const MAX_REACTION_KEY_LEN: usize = 64;

/// The reactions of a channel's outbox entries by target message id (rules
/// R1–R3): per (reactor, target, emoji) the reactor's latest op by its
/// stream seq wins and a repeat changes nothing (R1); a key over
/// [`MAX_REACTION_KEY_LEN`] is dropped and a message counts its first
/// [`MAX_REACTIONS_PER_MESSAGE`] emojis by first add (R2); the aggregate is
/// the reader's own, from the reactors' outboxes (R3), and each group names
/// its reactors (R4).
#[must_use]
pub fn build_reaction_groups(entries: &[OutboxItem]) -> HashMap<String, Vec<ReactionGroup>> {
    // (target, emoji, reactor) -> (stream seq, added, committed at)
    let mut latest: HashMap<(String, String, String), (u64, bool, u64)> = HashMap::new();
    for item in entries {
        let ChannelRecordEntry::Reaction(reaction) = &item.entry else {
            continue;
        };
        if reaction.expression.is_empty() || reaction.expression.len() > MAX_REACTION_KEY_LEN {
            continue;
        }
        let key = (
            reaction.message_id.clone(),
            reaction.expression.clone(),
            hex::encode(item.author.0),
        );
        let replace = (latest.get(&key)).is_none_or(|(seq, _, _)| item.stream_seq > *seq);
        if replace {
            latest.insert(key, (item.stream_seq, reaction.added, item.committed_at));
        }
    }
    // (target, emoji) -> (first add, reactors)
    let mut grouped: BTreeMap<(String, String), ((u64, String), Vec<String>)> = BTreeMap::new();
    for ((target, emoji, reactor), (_, added, at)) in latest {
        if !added {
            continue;
        }
        let group =
            (grouped.entry((target, emoji))).or_insert_with(|| ((at, reactor.clone()), Vec::new()));
        group.0 = group.0.clone().min((at, reactor.clone()));
        group.1.push(reactor);
    }
    let mut by_message: HashMap<String, Vec<((u64, String), ReactionGroup)>> = HashMap::new();
    for ((target, emoji), (first, mut reactors)) in grouped {
        reactors.sort();
        let group = ReactionGroup {
            emoji,
            count: u32::try_from(reactors.len()).unwrap_or(u32::MAX),
            reactors,
        };
        by_message.entry(target).or_default().push((first, group));
    }
    by_message
        .into_iter()
        .map(|(target, mut groups)| {
            groups.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.emoji.cmp(&b.1.emoji)));
            groups.truncate(MAX_REACTIONS_PER_MESSAGE);
            (target, groups.into_iter().map(|(_, group)| group).collect())
        })
        .collect()
}

pub fn build_poll_states<S: BuildHasher>(
    channel_entries: &[ChannelEntryItem],
    subkey_pseudonyms: &HashMap<u32, String, S>,
    my_pseudonym: &str,
) -> HashMap<String, MessagePoll> {
    let mut creates: BTreeMap<[u8; 16], PollCreateState> = BTreeMap::new();

    for item in channel_entries {
        match &item.entry {
            ChannelRecordEntry::PollCreate(create) => {
                let order = entry_order(item);
                let should_replace = match creates.get(&create.poll_id) {
                    None => true,
                    Some(existing) => {
                        existing.author_subkey == item.subkey_index
                            && existing.closed_at.is_none()
                            && order >= existing.order
                    }
                };
                if should_replace {
                    creates.insert(
                        create.poll_id,
                        PollCreateState {
                            order,
                            author_subkey: item.subkey_index,
                            message_id: create.message_id.clone(),
                            poll_id: create.poll_id,
                            question: create.question.clone(),
                            answers: create.answers.clone(),
                            multi_select: create.multi_select,
                            expires_at: create.expires_at,
                            closed_at: None,
                        },
                    );
                }
            }
            ChannelRecordEntry::PollClose(close) => {
                let Some(existing) = creates.get_mut(&close.poll_id) else {
                    continue;
                };
                if existing.author_subkey == item.subkey_index {
                    let order = entry_order(item);
                    let should_close = existing
                        .closed_at
                        .is_none_or(|closed_at| order >= closed_at);
                    if should_close {
                        existing.closed_at = Some(order);
                    }
                }
            }
            _ => {}
        }
    }

    let mut latest_votes: HashMap<([u8; 16], String), (EntryOrder, Vec<u8>)> = HashMap::new();
    for item in channel_entries {
        let ChannelRecordEntry::PollVote(vote) = &item.entry else {
            continue;
        };
        let Some(voter_pseudonym) = subkey_pseudonyms.get(&item.subkey_index) else {
            continue;
        };
        let Some(create) = creates.get(&vote.poll_id) else {
            continue;
        };
        let order = entry_order(item);
        if create.closed_at.is_some_and(|closed_at| order > closed_at) {
            continue;
        }
        let selected_answers = sanitize_selected_answers(
            vote.selected_answers.clone(),
            create.answers.len(),
            create.multi_select,
        );
        let key = (vote.poll_id, voter_pseudonym.clone());
        let replace = latest_votes
            .get(&key)
            .is_none_or(|(existing_order, _)| order >= *existing_order);
        if replace {
            latest_votes.insert(key, (order, selected_answers));
        }
    }

    let mut polls_by_message: BTreeMap<String, (EntryOrder, MessagePoll)> = BTreeMap::new();
    for (poll_id, create) in creates {
        let mut voters_by_answer: Vec<BTreeSet<String>> =
            vec![BTreeSet::new(); create.answers.len()];
        let mut my_selected_answers = Vec::new();

        for ((vote_poll_id, voter_pseudonym), (_, selected_answers)) in &latest_votes {
            if *vote_poll_id != poll_id {
                continue;
            }
            if voter_pseudonym == my_pseudonym {
                my_selected_answers.clone_from(selected_answers);
            }
            for answer_index in selected_answers {
                if let Some(voters) = voters_by_answer.get_mut(usize::from(*answer_index)) {
                    voters.insert(voter_pseudonym.clone());
                }
            }
        }

        let answers = create
            .answers
            .iter()
            .enumerate()
            .map(|(idx, text)| {
                let voters = voters_by_answer.get(idx).cloned().unwrap_or_default();
                let voters: Vec<String> = voters.into_iter().collect();
                MessagePollAnswer {
                    index: u8::try_from(idx).unwrap_or(u8::MAX),
                    text: text.clone(),
                    vote_count: u32::try_from(voters.len()).unwrap_or(u32::MAX),
                    voters,
                }
            })
            .collect();

        let materialized = MessagePoll {
            poll_id: hex::encode(create.poll_id),
            question: create.question,
            answers,
            multi_select: create.multi_select,
            expires_at: create.expires_at,
            closed: create.closed_at.is_some(),
            selected_answers: my_selected_answers,
        };

        let replace = polls_by_message
            .get(&create.message_id)
            .is_none_or(|(existing_order, _)| create.order >= *existing_order);
        if replace {
            polls_by_message.insert(create.message_id, (create.order, materialized));
        }
    }

    polls_by_message
        .into_iter()
        .map(|(message_id, (_, poll))| (message_id, poll))
        .collect()
}

fn entry_order(item: &ChannelEntryItem) -> EntryOrder {
    EntryOrder {
        lamport: item.entry.lamport(),
        subkey_index: item.subkey_index,
    }
}

fn sanitize_selected_answers(
    selected_answers: Vec<u8>,
    answer_count: usize,
    multi_select: bool,
) -> Vec<u8> {
    let mut answers: Vec<u8> = selected_answers
        .into_iter()
        .filter(|answer| usize::from(*answer) < answer_count)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    answers.sort_unstable();
    if multi_select {
        answers
    } else {
        answers.into_iter().take(1).collect()
    }
}

#[cfg(test)]
mod reaction_tests {
    use super::{build_reaction_groups, MAX_REACTIONS_PER_MESSAGE, MAX_REACTION_KEY_LEN};
    use crate::deps::OutboxItem;
    use rekindle_codec::community::channel_record::{ChannelReaction, ChannelRecordEntry};
    use rekindle_types::id::PseudonymKey;

    fn reaction(author: u8, seq: u64, target: &str, emoji: &str, added: bool) -> OutboxItem {
        OutboxItem {
            author: PseudonymKey([author; 32]),
            stream_seq: seq,
            committed_at: seq * 1_000,
            entry: ChannelRecordEntry::Reaction(ChannelReaction {
                message_id: target.into(),
                expression: emoji.into(),
                added,
                lamport: seq,
            }),
        }
    }

    /// R1: a reactor's latest op by stream seq wins wherever it sits in the
    /// read, and a repeat changes nothing.
    #[test]
    fn the_latest_op_per_reactor_wins_by_stream_seq() {
        let groups = build_reaction_groups(&[
            reaction(1, 5, "m", "tada", false),
            reaction(1, 3, "m", "tada", true),
            reaction(2, 1, "m", "tada", true),
            reaction(2, 2, "m", "tada", true),
        ]);
        let tada = &groups["m"][0];
        assert_eq!(tada.reactors, vec![hex::encode([2u8; 32])]);
        assert_eq!(tada.count, 1);
    }

    /// R2: an over-long key is dropped and a message counts at most
    /// `MAX_REACTIONS_PER_MESSAGE` emojis, the first added.
    #[test]
    fn readers_cap_the_key_length_and_distinct_emojis() {
        let mut items = vec![reaction(
            1,
            1,
            "m",
            &"x".repeat(MAX_REACTION_KEY_LEN + 1),
            true,
        )];
        items.extend(
            (0..=MAX_REACTIONS_PER_MESSAGE as u64)
                .map(|i| reaction(2, i + 10, "m", &format!("e{i:02}"), true)),
        );
        let groups = &build_reaction_groups(&items)["m"];
        assert_eq!(groups.len(), MAX_REACTIONS_PER_MESSAGE);
        assert!(groups.iter().all(|g| g.emoji.starts_with('e')));
        assert!(!groups
            .iter()
            .any(|g| g.emoji == format!("e{MAX_REACTIONS_PER_MESSAGE:02}")));
    }
}

#[cfg(test)]
mod tests {
    use super::build_poll_states;
    use crate::deps::ChannelEntryItem;
    use rekindle_codec::community::channel_record::{
        ChannelPollClose, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
    };
    use std::collections::HashMap;

    #[test]
    fn poll_votes_use_latest_vote_before_close() {
        let poll_id = [7u8; 16];
        let entries = vec![
            ChannelEntryItem {
                subkey_index: 1,
                seq: 0,
                slot_digest: [0; 32],
                author: rekindle_types::id::PseudonymKey([0; 32]),
                entry: ChannelRecordEntry::PollCreate(ChannelPollCreate {
                    poll_id,
                    message_id: "msg-1".into(),
                    question: "Pick one".into(),
                    answers: vec!["A".into(), "B".into()],
                    multi_select: false,
                    expires_at: None,
                    lamport: 10,
                }),
            },
            ChannelEntryItem {
                subkey_index: 2,
                seq: 0,
                slot_digest: [0; 32],
                author: rekindle_types::id::PseudonymKey([0; 32]),
                entry: ChannelRecordEntry::PollVote(ChannelPollVote {
                    poll_id,
                    selected_answers: vec![0],
                    lamport: 11,
                }),
            },
            ChannelEntryItem {
                subkey_index: 2,
                seq: 0,
                slot_digest: [0; 32],
                author: rekindle_types::id::PseudonymKey([0; 32]),
                entry: ChannelRecordEntry::PollVote(ChannelPollVote {
                    poll_id,
                    selected_answers: vec![1],
                    lamport: 12,
                }),
            },
            ChannelEntryItem {
                subkey_index: 1,
                seq: 0,
                slot_digest: [0; 32],
                author: rekindle_types::id::PseudonymKey([0; 32]),
                entry: ChannelRecordEntry::PollClose(ChannelPollClose {
                    poll_id,
                    lamport: 13,
                }),
            },
            ChannelEntryItem {
                subkey_index: 2,
                seq: 0,
                slot_digest: [0; 32],
                author: rekindle_types::id::PseudonymKey([0; 32]),
                entry: ChannelRecordEntry::PollVote(ChannelPollVote {
                    poll_id,
                    selected_answers: vec![0],
                    lamport: 14,
                }),
            },
        ];
        let subkeys = HashMap::from([(1_u32, "author".to_string()), (2_u32, "voter".to_string())]);

        let polls = build_poll_states(&entries, &subkeys, "voter");
        let poll = polls.get("msg-1").unwrap();

        assert!(poll.closed);
        assert_eq!(poll.selected_answers, vec![1]);
        assert_eq!(poll.answers[0].vote_count, 0);
        assert_eq!(poll.answers[1].vote_count, 1);
        assert_eq!(poll.answers[1].voters, vec!["voter".to_string()]);
    }
}
