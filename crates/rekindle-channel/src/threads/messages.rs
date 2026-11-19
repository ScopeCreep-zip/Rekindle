//! Thread message send/load.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::message::{ChannelPost, PostEntry};

use super::policy::ThreadMessageView;
use super::support::{open_thread_body, thread_replies, thread_scope, uuid_simple};
use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// A thread reply we sent: what a receiver's `ThreadMessagePosted`
/// carries besides the body, so our own frontends get the same event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadReplySent {
    pub message_id: String,
    pub sender_pseudonym: String,
    /// Seconds, as the `ThreadMessageReceived` envelope carries it.
    pub timestamp: u64,
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
/// The thread is unknown, we hold no pseudonym, or the queue or the mesh
/// send failed.
pub async fn send_thread_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<ThreadReplySent, ChannelError> {
    let thread = deps
        .thread_state(community_id, thread_id)
        .ok_or_else(|| ChannelError::Adapter("thread not found".into()))?;
    let sender_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let sent_at_ms = deps.now_secs().saturating_mul(1000);
    let sent = ThreadReplySent {
        message_id: format!("tmsg_{}", uuid_simple()),
        sender_pseudonym,
        timestamp: sent_at_ms / 1000,
    };
    let post = ChannelPost {
        community: community_id.to_owned(),
        channel: thread_id.to_owned(),
        message: sent.message_id.clone(),
        entry: PostEntry::ThreadReply {
            parent_channel: thread.parent_channel_id_hex.clone(),
            body: body.to_owned(),
            sent_at_ms,
            revives: thread.archived_op.clone(),
        },
    };
    deps.queue_post(&post).await?;
    let envelope = CommunityEnvelope::Control(ControlPayload::ThreadMessageReceived {
        thread_id: thread_id.to_string(),
        message_id: sent.message_id.clone(),
        sender_pseudonym: sent.sender_pseudonym.clone(),
        timestamp: sent.timestamp,
        reply_to_id: None,
    });
    deps.send_to_mesh(community_id, &envelope)?;
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
