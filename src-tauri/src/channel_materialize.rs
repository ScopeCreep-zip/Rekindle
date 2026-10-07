use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::commands::chat::{Message, MessagePoll, MessagePollAnswer, ReactionGroup};
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;
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

/// Open a channel message body: exactly the generation it names of the
/// channel's text key (plan D6), bound to the record, subkey and Lamport
/// position it was read from (architecture §8). There is no other key and
/// no decrypt without the position — a body that does not open there is
/// `decryption_failed`, and a generation we do not hold stays unreadable
/// until the key arrives.
pub(crate) fn decrypt_channel_record_message(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    mek_generation: u64,
    ciphertext: &[u8],
    at: rekindle_secrets::channel_body::BodyPosition<'_>,
) -> DecryptedMessageBody {
    let opened = rekindle_types::id::ChannelId::from_hex(channel_id).and_then(|channel| {
        let keys = state_helpers::key_provider(state);
        let scope = keys.scope_for_text(community_id, channel);
        let key = keys.key(
            community_id,
            scope,
            rekindle_types::channel_keys::KeyEpoch(mek_generation),
        )?;
        rekindle_secrets::channel_body::decrypt_channel_body(&key, at, ciphertext).ok()
    });
    match opened {
        Some(bytes) => DecryptedMessageBody {
            body: String::from_utf8_lossy(&bytes).into_owned(),
            decryption_failed: false,
        },
        None => DecryptedMessageBody {
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

pub(crate) async fn load_channel_messages_from_smpl(
    state: &SharedState,
    pool: &Db,
    community_id: &str,
    channel_id: &str,
    before_timestamp: Option<u64>,
    limit: u32,
) -> Result<Vec<Message>, String> {
    use rekindle_protocol::dht::community::channel_record::read_all_channel_entries;

    let (channel_key, my_pseudonym) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        (
            community.channel_log_keys.get(channel_id).cloned(),
            community.my_pseudonym_key.clone().unwrap_or_default(),
        )
    };
    let Some(channel_key) = channel_key else {
        return Ok(Vec::new());
    };
    // Logged out: no history to read.
    let Ok(record_pool) = state_helpers::record_pool(state) else {
        return Ok(Vec::new());
    };

    // The writer index: which slots to read, and whose each slot is
    // (plan C7.12).
    let subkey_pseudonyms: HashMap<u32, String> =
        crate::services::community::writers::writer_slots(state, pool, community_id)
            .await?
            .into_iter()
            .collect();
    let slots: Vec<u32> = subkey_pseudonyms.keys().copied().collect();
    let channel_entries = read_all_channel_entries(&record_pool, &channel_key, &slots)
        .await
        .map_err(|e| format!("read SMPL channel history: {e}"))?;
    let reaction_groups = build_reaction_groups(&channel_entries, &subkey_pseudonyms);
    let poll_states = build_poll_states(&channel_entries, &subkey_pseudonyms, &my_pseudonym);
    let mut filtered: Vec<(
        u32,
        rekindle_protocol::dht::community::channel_record::ChannelMessage,
    )> = channel_entries
        .iter()
        .filter_map(|item| match &item.entry {
            ChannelRecordEntry::Message(message)
                if before_timestamp.is_none_or(|before| message.timestamp < before) =>
            {
                Some((item.subkey_index, message.clone()))
            }
            _ => None,
        })
        .collect();
    if filtered.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
        let start = filtered.len() - usize::try_from(limit).unwrap_or(filtered.len());
        filtered = filtered.split_off(start);
    }

    if filtered.is_empty() {
        return Ok(Vec::new());
    }

    let hydrated_messages: Vec<Message> = filtered
        .iter()
        .map(|(subkey_index, message)| {
            let decrypted = decrypt_channel_record_message(
                state,
                community_id,
                channel_id,
                message.mek_generation,
                &message.ciphertext,
                rekindle_secrets::channel_body::BodyPosition {
                    channel_record_key: &channel_key,
                    subkey_index: *subkey_index,
                    lamport_ts: message.lamport_ts,
                },
            );
            Message {
                id: 0,
                is_own: message.sender_pseudonym == my_pseudonym,
                sender_id: message.sender_pseudonym.clone(),
                body: decrypted.body,
                decryption_failed: decrypted.decryption_failed,
                automod_blurred: false,
                timestamp: i64::try_from(message.timestamp).unwrap_or(i64::MAX),
                server_message_id: message.message_id.clone(),
                reactions: message
                    .message_id
                    .as_ref()
                    .and_then(|message_id| reaction_groups.get(message_id).cloned()),
                pinned: None,
                poll: message
                    .message_id
                    .as_ref()
                    .and_then(|message_id| poll_states.get(message_id).cloned()),
                forwarded_from_author: None,
                attachment: None,
                flags: 0,
            }
        })
        .collect();

    if let Ok(owner_key) = state_helpers::current_owner_key(state) {
        let chid = channel_id.to_string();
        let messages_for_db = filtered.clone();
        let owner_key_for_db = owner_key.clone();
        let state_for_db = state.clone();
        let community_id_for_db = community_id.to_string();
        let channel_id_for_db = channel_id.to_string();
        let channel_record_key_for_db = channel_key.clone();
        let _ = db_call(pool, move |conn| {
            for (subkey_index, message) in &messages_for_db {
                let Some(message_id) = message.message_id.as_deref() else {
                    continue;
                };
                let decrypted = decrypt_channel_record_message(
                    &state_for_db,
                    &community_id_for_db,
                    &channel_id_for_db,
                    message.mek_generation,
                    &message.ciphertext,
                    rekindle_secrets::channel_body::BodyPosition {
                        channel_record_key: &channel_record_key_for_db,
                        subkey_index: *subkey_index,
                        lamport_ts: message.lamport_ts,
                    },
                );
                let _ = crate::message_repo::insert_channel_message_with_protocol_metadata(
                    conn,
                    &owner_key_for_db,
                    &chid,
                    &message.sender_pseudonym,
                    &decrypted.body,
                    i64::try_from(message.timestamp).unwrap_or(i64::MAX),
                    true,
                    Some(i64::try_from(message.mek_generation).unwrap_or(i64::MAX)),
                    message_id,
                    message.lamport_ts,
                    false,
                );
            }
            Ok(())
        })
        .await;
    }

    Ok(hydrated_messages)
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
