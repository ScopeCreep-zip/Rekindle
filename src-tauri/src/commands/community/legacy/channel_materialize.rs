use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::commands::chat::{MessagePoll, MessagePollAnswer, ReactionGroup};
use crate::state::SharedState;
use rekindle_protocol::dht::community::channel_record::{ChannelRecordEntry, ChannelRecordItem};

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

pub(crate) struct DecryptedMessageBody {
    pub body: String,
    pub decryption_failed: bool,
}

pub(crate) fn build_reaction_groups(
    channel_entries: &[ChannelRecordItem],
    subkey_pseudonyms: &HashMap<u32, String>,
) -> HashMap<String, Vec<ReactionGroup>> {
    let mut latest_by_reactor: HashMap<(String, String, String), (u64, bool)> = HashMap::new();
    for item in channel_entries {
        let ChannelRecordEntry::Reaction(reaction) = &item.entry else {
            continue;
        };
        let Some(reactor_pseudonym) = subkey_pseudonyms.get(&item.subkey_index) else {
            continue;
        };
        let key = (
            reaction.message_id.clone(),
            reaction.expression.clone(),
            reactor_pseudonym.clone(),
        );
        let replace = latest_by_reactor
            .get(&key)
            .is_none_or(|(lamport, _)| reaction.lamport >= *lamport);
        if replace {
            latest_by_reactor.insert(key, (reaction.lamport, reaction.added));
        }
    }

    let mut grouped: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for ((message_id, expression, reactor_pseudonym), (_, added)) in latest_by_reactor {
        if added {
            grouped
                .entry((message_id, expression))
                .or_default()
                .push(reactor_pseudonym);
        }
    }

    let mut by_message = HashMap::new();
    for ((message_id, expression), mut reactors) in grouped {
        reactors.sort();
        by_message
            .entry(message_id)
            .or_insert_with(Vec::new)
            .push(ReactionGroup {
                emoji: expression,
                count: u32::try_from(reactors.len()).unwrap_or(u32::MAX),
                reactors,
            });
    }
    by_message
}

pub(crate) fn build_poll_states(
    channel_entries: &[ChannelRecordItem],
    subkey_pseudonyms: &HashMap<u32, String>,
    my_pseudonym: &str,
) -> HashMap<String, MessagePoll> {
    let mut creates: HashMap<[u8; 16], PollCreateState> = HashMap::new();

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

/// Architecture §8 line 1626 — context required to reconstruct the
/// AAD that the sender bound when encrypting. None of these are
/// secret; they're derived from the SMPL channel record + the
/// inbound `ChannelMessage`'s lamport timestamp.
#[derive(Clone, Copy)]
pub(crate) struct ChannelDecryptContext<'a> {
    pub channel_record_key: Option<&'a str>,
    pub subkey_index: u32,
    pub lamport_ts: u64,
}

pub(crate) fn decrypt_channel_record_message(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    mek_generation: u64,
    ciphertext: &[u8],
    ctx: ChannelDecryptContext<'_>,
) -> DecryptedMessageBody {
    let aad = ctx.channel_record_key.map(|key| {
        rekindle_crypto::group::media_key::ChannelAad {
            channel_record_key: key.as_bytes(),
            subkey_index: ctx.subkey_index,
            lamport_ts: ctx.lamport_ts,
        }
    });
    let try_decrypt = |mek: &rekindle_crypto::group::media_key::MediaEncryptionKey| {
        if let Some(aad) = aad {
            if let Ok(bytes) = mek.decrypt_with_aad(ciphertext, aad) {
                return Some(bytes);
            }
        }
        // Architecture §8 fallback for legacy messages written before AAD.
        mek.decrypt(ciphertext).ok()
    };
    {
        let channel_mek_cache = state.channel_mek_cache.lock();
        if let Some(mek) =
            channel_mek_cache.get(&(community_id.to_string(), channel_id.to_string()))
        {
            if mek.generation() == mek_generation {
                if let Some(bytes) = try_decrypt(mek) {
                    return DecryptedMessageBody {
                        body: String::from_utf8_lossy(&bytes).into_owned(),
                        decryption_failed: false,
                    };
                }
                return DecryptedMessageBody {
                    body: String::new(),
                    decryption_failed: true,
                };
            }
        }
    }

    let mek_cache = state.mek_cache.lock();
    match mek_cache.get(community_id) {
        Some(mek) if mek.generation() == mek_generation => match try_decrypt(mek) {
            Some(bytes) => DecryptedMessageBody {
                body: String::from_utf8_lossy(&bytes).into_owned(),
                decryption_failed: false,
            },
            None => DecryptedMessageBody {
                body: String::new(),
                decryption_failed: true,
            },
        },
        Some(_) | None => DecryptedMessageBody {
            body: String::new(),
            decryption_failed: true,
        },
    }
}

fn entry_order(item: &ChannelRecordItem) -> EntryOrder {
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
mod tests {
    use super::build_poll_states;
    use rekindle_protocol::dht::community::channel_record::{
        ChannelPollClose, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry, ChannelRecordItem,
    };
    use std::collections::HashMap;

    #[test]
    fn poll_votes_use_latest_vote_before_close() {
        let poll_id = [7u8; 16];
        let entries = vec![
            ChannelRecordItem {
                subkey_index: 1,
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
            ChannelRecordItem {
                subkey_index: 2,
                entry: ChannelRecordEntry::PollVote(ChannelPollVote {
                    poll_id,
                    selected_answers: vec![0],
                    lamport: 11,
                }),
            },
            ChannelRecordItem {
                subkey_index: 2,
                entry: ChannelRecordEntry::PollVote(ChannelPollVote {
                    poll_id,
                    selected_answers: vec![1],
                    lamport: 12,
                }),
            },
            ChannelRecordItem {
                subkey_index: 1,
                entry: ChannelRecordEntry::PollClose(ChannelPollClose {
                    poll_id,
                    lamport: 13,
                }),
            },
            ChannelRecordItem {
                subkey_index: 2,
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
