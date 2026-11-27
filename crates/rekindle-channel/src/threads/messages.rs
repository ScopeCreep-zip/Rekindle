//! Thread message send/load.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::message::SendState;
use rekindle_types::message::{ChannelPost, PostEntry};

use super::policy::ThreadMessageView;
use super::support::{open_thread_body, thread_replies, thread_scope, uuid_simple};
use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// A thread reply we sent: what a receiver's `ThreadMessagePosted`
/// carries besides the body, so our own frontends get the same event, and
/// its delivery state (`Queued`: the outbox moves it on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadReplySent {
    pub message_id: String,
    pub sender_pseudonym: String,
    /// Seconds, as the `ThreadMessageReceived` envelope carries it.
    pub timestamp: u64,
    pub send_state: SendState,
}

/// A reply to queue on a thread's stream.
pub(super) struct ThreadReplyDraft<'a> {
    pub thread_id: &'a str,
    pub parent_channel: &'a str,
    pub revives: Option<String>,
    pub message_id: String,
    pub body: &'a str,
}

const THREAD_REPLY_ID_PREFIX: &str = "tmsg_";

/// Whether our outbox client id identifies a thread reply.
pub fn is_thread_reply_id(id: &str) -> bool {
    id.strip_prefix(THREAD_REPLY_ID_PREFIX)
        .and_then(rekindle_types::id::ChannelId::from_hex)
        .is_some()
}

/// A new thread reply's id.
pub(super) fn new_reply_id() -> String {
    format!("{THREAD_REPLY_ID_PREFIX}{}", uuid_simple())
}

/// Send a thread reply (plan E3.3): an entry of our channel outbox
/// (`M`/4) on the thread's stream (`<community>:<thread>`), its body sealed
/// when the stream drains it under the parent channel's text key at
/// `{anchor, thread, author, streamSeq}`. The `ThreadMessageReceived`
/// envelope tells online members the reply exists; each reads it from our
/// outbox (the envelope carries no body: its position is fixed only at the
/// drain).
///
/// # Errors
/// The thread is unknown, we hold no pseudonym, or the queue failed. A
/// mesh failure after the queue is not an error: the reply is `Queued`.
pub async fn send_thread_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<ThreadReplySent, ChannelError> {
    let thread = deps
        .thread_state(community_id, thread_id)
        .ok_or_else(|| ChannelError::Adapter("thread not found".into()))?;
    let reply = ThreadReplyDraft {
        thread_id,
        parent_channel: &thread.parent_channel_id_hex,
        revives: thread.archived_op.clone(),
        message_id: new_reply_id(),
        body,
    };
    let (post, prepared) = prepare_reply(deps, community_id, reply)?;
    send_prepared_reply(prepared, deps.queue_post(&post), |envelope| {
        deps.send_to_mesh(community_id, envelope)
    })
    .await
}

pub(super) fn prepare_reply<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    reply: ThreadReplyDraft<'_>,
) -> Result<(ChannelPost, PreparedThreadReply), ChannelError> {
    let sender_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let timestamp = deps.now_secs();
    let post = ChannelPost {
        community: community_id.to_owned(),
        channel: reply.thread_id.to_owned(),
        message: reply.message_id.clone(),
        entry: PostEntry::ThreadReply {
            parent_channel: reply.parent_channel.to_owned(),
            body: reply.body.to_owned(),
            sent_at_ms: timestamp.saturating_mul(1000),
            revives: reply.revives,
        },
    };
    let prepared = PreparedThreadReply {
        thread_id: reply.thread_id.to_owned(),
        sent: ThreadReplySent {
            message_id: reply.message_id,
            sender_pseudonym,
            timestamp,
            send_state: SendState::Queued,
        },
    };
    Ok((post, prepared))
}

/// The local echo and mesh notice for a reply whose queue is about to commit.
pub struct PreparedThreadReply {
    pub(super) thread_id: String,
    pub(super) sent: ThreadReplySent,
}

/// Once the queue commits, the outbox owns delivery. The mesh notice is best effort.
///
/// # Errors
/// The durable queue failed.
pub async fn send_prepared_reply(
    prepared: PreparedThreadReply,
    queue: impl std::future::Future<Output = Result<(), ChannelError>>,
    notify: impl FnOnce(&CommunityEnvelope) -> Result<(), ChannelError>,
) -> Result<ThreadReplySent, ChannelError> {
    queue.await?;
    let sent = prepared.sent;
    let envelope = CommunityEnvelope::control(ControlPayload::ThreadMessageReceived {
        thread_id: prepared.thread_id,
        message_id: sent.message_id.clone(),
        sender_pseudonym: sent.sender_pseudonym.clone(),
        timestamp: sent.timestamp,
        reply_to_id: None,
    });
    if let Err(error) = notify(&envelope) {
        tracing::warn!(%error, "thread reply queued; the mesh notice was not sent");
    }
    Ok(sent)
}

/// Load a thread's replies from its authors' outboxes, each opened under
/// its exact generation at its position, newest `limit` (at most 200) older
/// than `before_timestamp_secs`, oldest first; in each author's stream
/// order, authors by pseudonym at equal times.
///
/// # Errors
/// The thread is unknown.
pub async fn load_thread_messages<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    limit: u32,
    before_timestamp_secs: Option<u64>,
) -> Result<Vec<ThreadMessageView>, ChannelError> {
    let thread = deps
        .thread_state(community_id, thread_id)
        .ok_or_else(|| ChannelError::Adapter("thread not found".into()))?;
    let Some(scope) = thread_scope(deps, community_id, &thread.parent_channel_id_hex) else {
        return Ok(Vec::new());
    };
    let mut items = thread_replies(deps, community_id, thread_id).await;
    items.sort_by(|a, b| {
        (a.1.timestamp, &a.1.sender_pseudonym, a.0.stream_seq).cmp(&(
            b.1.timestamp,
            &b.1.sender_pseudonym,
            b.0.stream_seq,
        ))
    });
    let before_ms = before_timestamp_secs.map_or(u64::MAX, |ts| ts.saturating_mul(1000));
    let my_pseudonym = deps.my_pseudonym_hex(community_id).unwrap_or_default();
    let mut messages: Vec<ThreadMessageView> = items
        .into_iter()
        .filter(|(_, message)| message.timestamp < before_ms)
        .rev()
        .take(limit.min(200) as usize)
        .map(|(item, message)| {
            let body = open_thread_body(deps, community_id, scope, thread_id, &item, &message);
            ThreadMessageView {
                is_own: message.sender_pseudonym == my_pseudonym,
                sender_pseudonym: message.sender_pseudonym.clone(),
                decryption_failed: body.is_none(),
                body: body.unwrap_or_default(),
                timestamp_ms: message.timestamp,
                server_message_id: message.message_id.clone(),
                mek_generation: message.mek_generation,
                stream_seq: item.stream_seq,
            }
        })
        .collect();
    messages.reverse();
    Ok(messages)
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod tests;
