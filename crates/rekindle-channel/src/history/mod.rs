//! A channel's history (plan E3.1 E2c): the messages of its record, opened
//! under the generation each names, with their reactions and polls, and the
//! merge of those with the host's stored rows. The host reads its rows
//! (`rekindle_db::repo::messages::channel`), calls [`read_channel_record`],
//! stores what it did not hold and answers with [`merge_message_lists`].

mod materialize;
mod poll_states;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_types::dto::channel_message::{
    ChannelMessageView, MessageAttachment, MessagePoll, ReactionGroup,
};
use rekindle_types::message::SendState;

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;
use crate::send::SlotBodyPosition;

pub use materialize::{build_reaction_groups, MAX_REACTIONS_PER_MESSAGE, MAX_REACTION_KEY_LEN};
pub use poll_states::{build_poll_states, poll_tallies, PollTally};

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
    /// The attachment's offer embedded in the message, which the host
    /// stores as the attachment's record.
    pub attachment: Option<rekindle_types::attachment::AttachmentOffer>,
    /// The channel record it was read from.
    pub record_key: String,
    /// The seq of its author's slot value it was read from: the gate keeps
    /// a removed author's post by it (A/B finding R3-8).
    pub slot_seq: u32,
    /// The digest of that slot value's page: the gate keeps a departed
    /// author's post only from the exact value its departure named (S9).
    pub slot_digest: [u8; 32],
}

/// An edit or a delete in the channel record (plan E3.3), with the author
/// whose signed slot value carries it and where: the host applies it under
/// `crate::changes` (E1–E3).
#[derive(Debug, Clone)]
pub struct RecordChange {
    pub record_key: String,
    pub subkey_index: u32,
    pub author: rekindle_types::id::PseudonymKey,
    pub entry: ChannelRecordEntry,
}

/// What a record read returns: its messages, the authors' changes, and
/// the reactions and polls the record holds by target message id. A
/// target need not be a message of the record: a post on its author's
/// channel outbox (plan E3.3) is a stored row, and the reactions and polls
/// written to the record still name it.
#[derive(Debug, Clone, Default)]
pub struct ChannelRecordRead {
    pub messages: Vec<RecordMessage>,
    pub changes: Vec<RecordChange>,
    pub reactions: HashMap<String, Vec<ReactionGroup>>,
    pub polls: HashMap<String, MessagePoll>,
}

/// The latest `limit` messages of `channel_id`'s record (only those older
/// than `before` when given), in channel order: by Lamport time, ties by
/// sender, as every peer orders a channel. A body that does not open under
/// the generation it names is `decryption_failed`. A channel with no record
/// yet has no messages. Every edit and delete in the record comes with them,
/// in the authors' seq order.
///
/// # Errors
/// The record read failed.
pub async fn read_channel_record<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    before: Option<u64>,
    limit: u32,
) -> Result<ChannelRecordRead, ChannelError> {
    // Reactions and polls ride their authors' outboxes (plan E3.3).
    let outbox = deps.read_outbox_entries(community_id, channel_id).await;
    let me = deps.my_pseudonym_hex(community_id).unwrap_or_default();
    let reactions = build_reaction_groups(&outbox);
    let polls = build_poll_states(&outbox, &me);
    let Some(record_key) = deps.channel_record_key(community_id, channel_id) else {
        return Ok(ChannelRecordRead {
            reactions,
            polls,
            ..ChannelRecordRead::default()
        });
    };
    let entries = deps
        .read_all_channel_entries(community_id, &record_key)
        .await?;
    let changes = (entries.iter())
        .filter(|item| {
            matches!(
                item.entry,
                ChannelRecordEntry::Edit(_) | ChannelRecordEntry::Delete(_)
            )
        })
        .map(|item| RecordChange {
            record_key: record_key.clone(),
            subkey_index: item.subkey_index,
            author: item.author.clone(),
            entry: item.entry.clone(),
        })
        .collect();
    let mut messages: Vec<_> = entries
        .iter()
        .filter_map(|item| match &item.entry {
            ChannelRecordEntry::Message(message)
                if before.is_none_or(|before| message.timestamp < before) =>
            {
                Some((item.subkey_index, (item.seq, item.slot_digest), message))
            }
            _ => None,
        })
        .collect();
    messages.sort_by(|a, b| {
        (a.2.lamport_ts, &a.2.sender_pseudonym).cmp(&(b.2.lamport_ts, &b.2.sender_pseudonym))
    });
    let keep = usize::try_from(limit).unwrap_or(usize::MAX);
    let skip = messages.len().saturating_sub(keep);
    let messages = messages
        .into_iter()
        .skip(skip)
        .map(|(subkey_index, (slot_seq, slot_digest), message)| {
            let body = crate::text_keys::open_text(
                deps,
                community_id,
                channel_id,
                message.mek_generation,
                SlotBodyPosition {
                    channel_record_key: &record_key,
                    subkey_index,
                    lamport_ts: message.lamport_ts,
                },
                &message.ciphertext,
            );
            let is_own = message.sender_pseudonym == me;
            let mentions_me = crate::mentions::message_mentions_me(deps, community_id, message);
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
                    attachment: message.attachment.as_ref().map(|offer| MessageAttachment {
                        attachment_id: hex::encode(offer.attachment_id),
                        filename: offer.filename.clone(),
                        mime_type: offer.mime_type.clone(),
                        total_size: offer.total_size,
                        chunk_count: offer.chunk_count,
                        downloaded: false,
                    }),
                    flags: 0,
                    send_state: if is_own {
                        SendState::Sent
                    } else {
                        SendState::Delivered
                    },
                    failure_reason: None,
                    edited_at: None,
                    removed_by: None,
                    history_gap_through: None,
                },
                reply_to: message.reply_to.clone(),
                mek_generation: message.mek_generation,
                lamport_ts: message.lamport_ts,
                mentions_me,
                attachment: message.attachment.clone(),
                record_key: record_key.clone(),
                slot_seq,
                slot_digest,
            }
        })
        .collect();
    Ok(ChannelRecordRead {
        messages,
        changes,
        reactions,
        polls,
    })
}

/// Merge `incoming` into `existing`, one message per message id (or, with
/// none, per timestamp, sender and body): a collision keeps the existing
/// row and takes the incoming one's id, reactions, pin and poll. An opened
/// copy is never marked undecryptable by one that did not open: our stored
/// rows were opened when stored, and a record read lacking that generation
/// says nothing about them (A/B R3-9); an opened copy clears a failed
/// record-only row and gives it its body. Sorted by timestamp, cut to the
/// latest `limit` (at least one).
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
    if existing.decryption_failed && !incoming.decryption_failed {
        existing.decryption_failed = false;
        existing.body.clone_from(&incoming.body);
    }
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

#[cfg(test)]
mod tests {
    use super::merge_message_lists;
    use rekindle_types::dto::channel_message::ChannelMessageView;
    use rekindle_types::message::SendState;

    fn view(id: &str, body: &str, decryption_failed: bool) -> ChannelMessageView {
        ChannelMessageView {
            id: 0,
            sender_id: "author".into(),
            body: body.into(),
            decryption_failed,
            automod_blurred: false,
            timestamp: 1,
            is_own: false,
            server_message_id: Some(id.into()),
            reactions: None,
            pinned: None,
            poll: None,
            forwarded_from_author: None,
            attachment: None,
            flags: 0,
            send_state: SendState::Delivered,
            failure_reason: None,
            edited_at: None,
            removed_by: None,
            history_gap_through: None,
        }
    }

    /// A record read that cannot open a body (its generation is not held)
    /// never marks the row we stored when we opened it undecryptable.
    #[test]
    fn a_failed_copy_never_flips_an_opened_row() {
        let mut rows = vec![view("m1", "hello", false)];
        merge_message_lists(&mut rows, vec![view("m1", "", true)], 10);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].decryption_failed);
        assert_eq!(rows[0].body, "hello");
    }

    /// Two record copies of one message: the opened one wins, body and all.
    #[test]
    fn an_opened_copy_clears_a_failed_record_only_row() {
        let mut rows = Vec::new();
        merge_message_lists(
            &mut rows,
            vec![view("m1", "", true), view("m1", "hello", false)],
            10,
        );
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].decryption_failed);
        assert_eq!(rows[0].body, "hello");
    }
}
