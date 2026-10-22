//! A channel's history (plan E3.1 E2c): the messages of its record, opened
//! under the generation each names, with their reactions and polls, and the
//! merge of those with the host's stored rows. The host reads its rows
//! (`rekindle_db::repo::messages::channel`), calls [`read_channel_record`],
//! stores what it did not hold and answers with [`merge_message_lists`].

mod materialize;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_types::dto::channel_message::{ChannelMessageView, ReactionGroup};
use rekindle_types::message::SendState;

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;
use crate::send::BodyPosition;

pub use materialize::{build_poll_states, build_reaction_groups};

/// One message of the channel record, as read: its view, and what the host
/// stores it with.
#[derive(Debug, Clone)]
pub struct RecordMessage {
    pub view: ChannelMessageView,
    /// The message id of the parent it replies to.
    pub reply_to: Option<String>,
    pub mek_generation: u64,
    pub lamport_ts: u64,
    /// Whether it mentions us (an `@` of our pseudonym, `@everyone` or
    /// `@here`), for the unread mention count (plan E3.1 G1).
    pub mentions_me: bool,
}

/// The latest `limit` messages of `channel_id`'s record (only those older
/// than `before` when given), in channel order: by Lamport time, ties by
/// sender, as every peer orders a channel. A body that does not open under
/// the generation it names is `decryption_failed`. A channel with no record
/// yet has no messages.
///
/// # Errors
/// The record read failed.
pub async fn read_channel_record<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    before: Option<u64>,
    limit: u32,
) -> Result<Vec<RecordMessage>, ChannelError> {
    let Some(record_key) = deps.channel_record_key(community_id, channel_id) else {
        return Ok(Vec::new());
    };
    let entries = deps
        .read_all_channel_entries(community_id, &record_key)
        .await?;
    let pseudonyms = deps.stage_pseudonyms_by_subkey(community_id).await?;
    let me = deps.my_pseudonym_hex(community_id).unwrap_or_default();
    let reactions = build_reaction_groups(&entries, &pseudonyms);
    let polls = build_poll_states(&entries, &pseudonyms, &me);
    let mut messages: Vec<_> = entries
        .iter()
        .filter_map(|item| match &item.entry {
            ChannelRecordEntry::Message(message)
                if before.is_none_or(|before| message.timestamp < before) =>
            {
                Some((item.subkey_index, message))
            }
            _ => None,
        })
        .collect();
    messages.sort_by(|a, b| {
        (a.1.lamport_ts, &a.1.sender_pseudonym).cmp(&(b.1.lamport_ts, &b.1.sender_pseudonym))
    });
    let keep = usize::try_from(limit).unwrap_or(usize::MAX);
    let skip = messages.len().saturating_sub(keep);
    Ok(messages
        .into_iter()
        .skip(skip)
        .map(|(subkey_index, message)| {
            let body = crate::text_keys::open_text(
                deps,
                community_id,
                channel_id,
                message.mek_generation,
                BodyPosition {
                    channel_record_key: &record_key,
                    subkey_index,
                    lamport_ts: message.lamport_ts,
                },
                &message.ciphertext,
            );
            let is_own = message.sender_pseudonym == me;
            let mentions_me = crate::mentions::message_mentions_me(deps, community_id, &message);
            let id = message.message_id.as_ref();
            RecordMessage {
                view: ChannelMessageView {
                    id: 0,
                    sender_id: message.sender_pseudonym.clone(),
                    decryption_failed: body.is_none(),
                    body: body
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default(),
                    automod_blurred: false,
                    timestamp: i64::try_from(message.timestamp).unwrap_or(i64::MAX),
                    is_own,
                    server_message_id: message.message_id.clone(),
                    reactions: id.and_then(|id| reactions.get(id).cloned()),
                    pinned: None,
                    poll: id.and_then(|id| polls.get(id).cloned()),
                    forwarded_from_author: None,
                    attachment: None,
                    flags: 0,
                    send_state: if is_own {
                        SendState::Sent
                    } else {
                        SendState::Delivered
                    },
                },
                reply_to: message.reply_to.clone(),
                mek_generation: message.mek_generation,
                lamport_ts: message.lamport_ts,
                mentions_me,
            }
        })
        .collect())
}

/// Merge `incoming` into `existing`, one message per message id (or, with
/// none, per timestamp, sender and body): a collision keeps the existing
/// row and takes the incoming one's id, failure flag, reactions, pin and
/// poll. Sorted by timestamp, cut to the latest `limit` (at least one).
pub fn merge_message_lists(
    existing: &mut Vec<ChannelMessageView>,
    incoming: Vec<ChannelMessageView>,
    limit: u32,
) {
    let mut seen: HashMap<String, usize> = existing
        .iter()
        .enumerate()
        .map(|(idx, message)| (identity(message), idx))
        .collect();
    for message in incoming {
        let key = identity(&message);
        if let Some(idx) = seen.get(&key).copied() {
            merge_fields(&mut existing[idx], &message);
        } else {
            seen.insert(key, existing.len());
            existing.push(message);
        }
    }
    existing.sort_by_key(|message| message.timestamp);
    let max_len = usize::try_from(limit.max(1)).unwrap_or(usize::MAX);
    let start = existing.len().saturating_sub(max_len);
    existing.drain(0..start);
}

fn merge_fields(existing: &mut ChannelMessageView, incoming: &ChannelMessageView) {
    if existing.server_message_id.is_none() {
        existing
            .server_message_id
            .clone_from(&incoming.server_message_id);
    }
    existing.decryption_failed |= incoming.decryption_failed;
    existing.reactions = merge_reactions(existing.reactions.take(), incoming.reactions.clone());
    if incoming.pinned == Some(true) {
        existing.pinned = Some(true);
    }
    if incoming.poll.is_some() {
        existing.poll.clone_from(&incoming.poll);
    }
}

fn merge_reactions(
    existing: Option<Vec<ReactionGroup>>,
    incoming: Option<Vec<ReactionGroup>>,
) -> Option<Vec<ReactionGroup>> {
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for reaction in existing
        .into_iter()
        .flatten()
        .chain(incoming.into_iter().flatten())
    {
        groups
            .entry(reaction.emoji)
            .or_default()
            .extend(reaction.reactors);
    }
    (!groups.is_empty()).then(|| {
        groups
            .into_iter()
            .map(|(emoji, reactors)| ReactionGroup {
                count: u32::try_from(reactors.len()).unwrap_or(u32::MAX),
                emoji,
                reactors: reactors.into_iter().collect(),
            })
            .collect()
    })
}

fn identity(message: &ChannelMessageView) -> String {
    match &message.server_message_id {
        Some(message_id) => format!("id:{message_id}"),
        None => format!(
            "ts:{}:{}:{}",
            message.timestamp, message.sender_id, message.body
        ),
    }
}
