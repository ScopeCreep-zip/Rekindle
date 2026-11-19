//! The `channel_post` stream's drain (plan E3.3): each queued post of one
//! channel becomes an entry of our member record's channel outbox (`M`/4).
//! Its body is read from our `messages` row and sealed now, at its
//! position `{anchor, channel, author, streamSeq}` under the newest text
//! generation held, so a key rotation while it waited cannot leave it under
//! an old key; the entry carries that generation and the channel tag
//! derived from the same key. A post whose channel key we do not hold yet
//! waits (the awaiting-key behaviour, plan E3.2 S7l): the drain stops at it
//! and the stream retries it. The batch is appended in queue order as
//! sealed pages ([`OutboxWriter::append`]). An edit or delete of our post
//! is an entry of the same stream (plan E3.3): an edit's new body is
//! sealed at the edit's own position, a delete has none. An attachment
//! message is a message entry carrying the offer its row holds, and a
//! downloaded file's possession an `AttachmentCached` entry (slice 7).

use rekindle_codec::community::channel_record::{
    ChannelAttachmentCached, ChannelDelete, ChannelEdit, ChannelForward, ChannelHandRaise,
    ChannelPollClose, ChannelPollCounted, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
};
use rekindle_types::attachment::AttachmentBitmap;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::ChannelId;
use rekindle_types::message::{ChannelPost, PostEntry};

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;
use crate::mentions::resolve_outbound_mentions;
use crate::outbox_chain::{seal_batch, OutboxCommit, OutboxWriter, PendingEntry};
use crate::outbox_seal::{seal_entry_body, SealedEntryBody};
use crate::send::{build_channel_message, BodyPosition, BuildChannelMessageParams};

/// One queued post of the stream and its frozen per-stream seq.
#[derive(Debug, Clone)]
pub struct DrainPost {
    pub post: ChannelPost,
    pub stream_seq: u64,
}

/// What the drain did with one post, in queue order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainOutcome {
    /// On our outbox chain at this chain seq.
    Sent(u64),
    /// Not appended now; the stream retries it (no key yet, a failed
    /// write).
    Deferred(String),
    /// It can never be appended (not ours, gone from history, over one
    /// page).
    Failed(String),
}

/// The body a post's entry seals, and the reply its message names.
struct StoredBody {
    text: String,
    reply_to: Option<String>,
}

/// The body `item` seals: an edit's new body, nothing for a delete, a
/// possession entry, a reaction, a poll op or a hand raise, else our stored post's (its row must
/// be ours).
async fn stored_body<D: ChannelMessagingDeps>(
    deps: &D,
    item: &DrainPost,
    author_hex: &str,
) -> Result<StoredBody, DrainOutcome> {
    let post = &item.post;
    // An edit or delete names its target; a post is its own row.
    let (target, body) = match &post.entry {
        PostEntry::Cached { .. }
        | PostEntry::Reaction { .. }
        | PostEntry::PollCreate { .. }
        | PostEntry::PollVote { .. }
        | PostEntry::PollClose { .. }
        | PostEntry::HandRaise { .. } => {
            return Ok(StoredBody {
                text: String::new(),
                reply_to: None,
            })
        }
        PostEntry::ThreadReply { body, .. } => {
            return Ok(StoredBody {
                text: body.clone(),
                reply_to: None,
            })
        }
        PostEntry::Edit { target, body, .. } => (target.as_str(), Some(body.as_str())),
        PostEntry::Delete { target, .. } => (target.as_str(), Some("")),
        _ => (post.message.as_str(), None),
    };
    let stored = deps
        .find_channel_message_by_id(&post.channel, target)
        .await
        .ok_or_else(|| DrainOutcome::Failed(format!("post {target} not in history")))?;
    if stored.sender_key != author_hex {
        return Err(DrainOutcome::Failed(format!("post {target} is not ours")));
    }
    Ok(StoredBody {
        text: body.map_or(stored.body, str::to_owned),
        reply_to: stored.reply_to,
    })
}

/// The entry `post` writes, its body sealed at `stream_seq`.
async fn pending_of<D: ChannelMessagingDeps>(
    deps: &D,
    writer: &OutboxWriter<'_>,
    item: &DrainPost,
    committed_at: u64,
) -> Result<PendingEntry, DrainOutcome> {
    let post = &item.post;
    let (community_id, channel_id) = (post.community.as_str(), post.channel.as_str());
    let author = writer.author.verifying_key().to_bytes();
    let author_hex = hex::encode(author);
    let body = stored_body(deps, item, &author_hex).await?;
    let at = BodyPosition {
        anchor: writer.anchor,
        channel_id,
        author: &author,
        stream_seq: item.stream_seq,
    };
    // A thread's stream is sealed under its parent channel's text scope.
    let scope_channel = match &post.entry {
        PostEntry::ThreadReply { parent_channel, .. } => parent_channel.as_str(),
        _ => channel_id,
    };
    let sealed = seal_at(deps, community_id, scope_channel, at, body.text.as_bytes())?;
    let timestamp_ms = i64::try_from(committed_at).unwrap_or(i64::MAX);
    let entry = match &post.entry {
        PostEntry::Message | PostEntry::Attachment { .. } | PostEntry::ThreadReply { .. } => {
            let timestamp_ms = match &post.entry {
                PostEntry::ThreadReply { sent_at_ms, .. } => {
                    i64::try_from(*sent_at_ms).unwrap_or(i64::MAX)
                }
                _ => timestamp_ms,
            };
            let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
                resolve_outbound_mentions(deps, community_id, &author_hex, &body.text);
            let mut message = build_channel_message(BuildChannelMessageParams {
                sequence: item.stream_seq,
                sender_pseudonym: author_hex,
                ciphertext: sealed.ciphertext,
                mek_generation: sealed.generation,
                timestamp_ms,
                lamport_ts: item.stream_seq,
                message_id: post.message.clone(),
                reply_to: body.reply_to.clone(),
                mention_flag_bits: mention_flags,
                mentioned_pseudonyms,
                mentioned_roles,
            });
            match &post.entry {
                PostEntry::Attachment { offer, flags } => {
                    message.attachment = Some(offer.clone());
                    message.flags |= flags;
                }
                PostEntry::ThreadReply { revives, .. } => message.revives.clone_from(revives),
                _ => {}
            }
            ChannelRecordEntry::Message(message)
        }
        PostEntry::Forward {
            original_channel,
            original_message,
            original_author,
        } => ChannelRecordEntry::Forward(ChannelForward {
            sequence: item.stream_seq,
            sender_pseudonym: author_hex,
            original_message_id: original_message.clone(),
            original_channel_id: original_channel.clone(),
            original_author: original_author.clone(),
            content_snapshot: sealed.ciphertext,
            mek_generation: sealed.generation,
            timestamp: committed_at,
            lamport_ts: item.stream_seq,
            message_id: Some(post.message.clone()),
        }),
        PostEntry::Edit {
            target,
            edited_at,
            seq,
            ..
        } => ChannelRecordEntry::Edit(ChannelEdit {
            message_id: target.clone(),
            ciphertext: sealed.ciphertext,
            mek_generation: sealed.generation,
            edited_at: *edited_at,
            lamport_ts: *seq,
        }),
        // Our possession of a downloaded file: a full bitmap, ordered by
        // its stream seq; no body, the tag still names its channel.
        PostEntry::Cached {
            attachment_id,
            chunk_count,
        } => ChannelRecordEntry::AttachmentCached(ChannelAttachmentCached {
            attachment_id: *attachment_id,
            chunk_bitmap: AttachmentBitmap::full(*chunk_count).as_bytes().to_vec(),
            chunk_count: *chunk_count,
            author_pseudonym: author_hex,
            lamport_ts: item.stream_seq,
        }),
        // A reaction op (rule R1), ordered by its stream seq; no body.
        PostEntry::Reaction {
            target,
            emoji,
            added,
        } => ChannelRecordEntry::Reaction(crate::reactions::build_reaction(
            target.clone(),
            emoji.clone(),
            *added,
            item.stream_seq,
        )),
        // Poll ops, ordered by their stream seq; no body.
        PostEntry::PollCreate {
            poll_id,
            target,
            question,
            answers,
            multi_select,
            expires_at,
        } => ChannelRecordEntry::PollCreate(ChannelPollCreate {
            poll_id: *poll_id,
            message_id: target.clone(),
            question: question.clone(),
            answers: answers.clone(),
            multi_select: *multi_select,
            expires_at: *expires_at,
            lamport: item.stream_seq,
        }),
        PostEntry::PollVote {
            poll_id,
            selected_answers,
        } => ChannelRecordEntry::PollVote(ChannelPollVote {
            poll_id: *poll_id,
            selected_answers: selected_answers.clone(),
            lamport: item.stream_seq,
        }),
        PostEntry::PollClose { poll_id, counted } => {
            ChannelRecordEntry::PollClose(ChannelPollClose {
                poll_id: *poll_id,
                lamport: item.stream_seq,
                counted: (counted.iter())
                    .map(|(voter, stream_seq)| ChannelPollCounted {
                        voter: voter.clone(),
                        stream_seq: *stream_seq,
                    })
                    .collect(),
            })
        }
        PostEntry::HandRaise { raised } => ChannelRecordEntry::HandRaise(ChannelHandRaise {
            raised: *raised,
            lamport: item.stream_seq,
        }),
        // A delete carries no body; the entry still names its channel by
        // the tag of the key it was drained under.
        PostEntry::Delete {
            target,
            deleted_at,
            seq,
        } => ChannelRecordEntry::Delete(ChannelDelete {
            message_id: target.clone(),
            deleted_at: *deleted_at,
            lamport_ts: *seq,
        }),
    };
    let pending = PendingEntry {
        channel_tag: sealed.channel_tag,
        stream_seq: item.stream_seq,
        generation: sealed.generation,
        entry,
    };
    // An entry no page holds alone can never be appended.
    if let Err(error) = seal_batch(
        None,
        writer.anchor,
        writer.author,
        committed_at,
        std::slice::from_ref(&pending),
    ) {
        return Err(DrainOutcome::Failed(error.to_string()));
    }
    Ok(pending)
}

/// Seal `body` at `at` under the current text key of `scope_channel` (the
/// entry's channel, or a thread's parent); a channel whose key we do not
/// hold yet defers the entry.
fn seal_at<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    scope_channel: &str,
    at: BodyPosition<'_>,
    body: &[u8],
) -> Result<SealedEntryBody, DrainOutcome> {
    let channel = ChannelId::from_hex(scope_channel)
        .ok_or_else(|| DrainOutcome::Failed(format!("{scope_channel} is not a channel")))?;
    let scope: KeyScope = deps.keys().scope_for_text(community_id, channel);
    match seal_entry_body(&*deps.keys(), community_id, scope, at, body) {
        Ok(sealed) => Ok(sealed),
        Err(ChannelError::MekMissing { .. }) => Err(DrainOutcome::Deferred(
            "the channel's key is not held yet".into(),
        )),
        Err(error) => Err(DrainOutcome::Deferred(error.to_string())),
    }
}

/// Drain `posts` (one channel's stream, in queue order) onto our outbox
/// chain: the outcomes of a prefix, ending at the first post that is not
/// appended now.
pub async fn drain_posts<D: ChannelMessagingDeps>(
    deps: &D,
    writer: &OutboxWriter<'_>,
    posts: &[DrainPost],
    committed_at: u64,
) -> Vec<DrainOutcome> {
    let mut pending = Vec::with_capacity(posts.len());
    let mut stopped = None;
    for item in posts {
        match pending_of(deps, writer, item, committed_at).await {
            Ok(entry) => pending.push(entry),
            Err(outcome) => {
                stopped = Some(outcome);
                break;
            }
        }
    }
    let report = writer.append(&pending, committed_at).await;
    let mut outcomes: Vec<DrainOutcome> = report
        .committed
        .iter()
        .map(|c| match c {
            OutboxCommit::Appended(seq) | OutboxCommit::Already(seq) => DrainOutcome::Sent(*seq),
        })
        .collect();
    if let Some(error) = report.error {
        tracing::debug!(%error, "channel outbox append stopped");
        outcomes.push(DrainOutcome::Deferred(error.to_string()));
        return outcomes;
    }
    outcomes.extend(stopped);
    outcomes
}
