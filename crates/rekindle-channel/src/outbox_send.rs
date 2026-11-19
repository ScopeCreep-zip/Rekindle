//! The `channel_post` stream's drain (plan E3.3): each queued post of one
//! channel becomes an entry of our member record's channel outbox (`M`/4).
//! Its body is read from our `messages` row and sealed now, at its
//! position `{anchor, channel, author, streamSeq}` under the newest text
//! generation held, so a key rotation while it waited cannot leave it under
//! an old key; the entry carries that generation and the channel tag
//! derived from the same key. A post whose channel key we do not hold yet
//! waits (the awaiting-key behaviour, plan E3.2 S7l): the drain stops at it
//! and the stream retries it. The batch is appended in queue order as
//! sealed pages ([`OutboxWriter::append`]).

use rekindle_codec::community::channel_record::{ChannelForward, ChannelRecordEntry};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::ChannelId;
use rekindle_types::message::{ChannelPost, PostEntry};

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;
use crate::mentions::resolve_outbound_mentions;
use crate::outbox_chain::{seal_batch, OutboxCommit, OutboxWriter, PendingEntry};
use crate::outbox_seal::seal_entry_body;
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

/// The entry `post` writes, its body sealed at `stream_seq`.
async fn pending_of<D: ChannelMessagingDeps>(
    deps: &D,
    writer: &OutboxWriter<'_>,
    item: &DrainPost,
    committed_at: u64,
) -> Result<PendingEntry, DrainOutcome> {
    let post = &item.post;
    let (community_id, channel_id) = (post.community.as_str(), post.channel.as_str());
    let channel = ChannelId::from_hex(channel_id)
        .ok_or_else(|| DrainOutcome::Failed(format!("{channel_id} is not a channel")))?;
    let scope: KeyScope = deps.keys().scope_for_text(community_id, channel);
    let author = writer.author.verifying_key().to_bytes();
    let author_hex = hex::encode(author);
    let stored = deps
        .find_channel_message_by_id(channel_id, &post.message)
        .await
        .ok_or_else(|| DrainOutcome::Failed(format!("post {} not in history", post.message)))?;
    if stored.sender_key != author_hex {
        return Err(DrainOutcome::Failed(format!(
            "post {} is not ours",
            post.message
        )));
    }
    let at = BodyPosition {
        anchor: writer.anchor,
        channel_id,
        author: &author,
        stream_seq: item.stream_seq,
    };
    let sealed = match seal_entry_body(
        &*deps.keys(),
        community_id,
        scope,
        at,
        stored.body.as_bytes(),
    ) {
        Ok(sealed) => sealed,
        Err(ChannelError::MekMissing { .. }) => {
            return Err(DrainOutcome::Deferred(
                "the channel's key is not held yet".into(),
            ))
        }
        Err(error) => return Err(DrainOutcome::Deferred(error.to_string())),
    };
    let timestamp_ms = i64::try_from(committed_at).unwrap_or(i64::MAX);
    let entry = match &post.entry {
        PostEntry::Message => {
            let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
                resolve_outbound_mentions(deps, community_id, &author_hex, &stored.body);
            ChannelRecordEntry::Message(build_channel_message(BuildChannelMessageParams {
                sequence: item.stream_seq,
                sender_pseudonym: author_hex,
                ciphertext: sealed.ciphertext,
                mek_generation: sealed.generation,
                timestamp_ms,
                lamport_ts: item.stream_seq,
                message_id: post.message.clone(),
                reply_to: stored.reply_to.clone(),
                mention_flag_bits: mention_flags,
                mentioned_pseudonyms,
                mentioned_roles,
            }))
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
        PostEntry::SlotAttachment { .. } => {
            return Err(DrainOutcome::Deferred(
                "an attachment post settles with its slot write".into(),
            ))
        }
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
