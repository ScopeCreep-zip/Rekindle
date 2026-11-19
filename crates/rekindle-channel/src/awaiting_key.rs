//! A post composed before we hold its channel's key (plan E3.2 S7l,
//! finding F9-1, decided from research 2026-11-17): it is **queued, not
//! refused**. Its outbox `channel_post` row waits in state `awaiting_key`,
//! keyed by its client id, its plaintext in its `messages` row (SQLCipher),
//! until the key is installed; then it is released (`queued`) in
//! composition order per channel, and its stream's sender seals and appends
//! it like any post ([`crate::outbox_send`], plan E3.3): sealed then under
//! the newest generation held (never at queue time, so a rotation while it
//! waited cannot leave it under an old key) and stamped with the commit time
//! (the composed time stays in the local row only). A later post of the
//! channel queues behind a waiting one; a post released or retried while
//! the key is absent again is deferred by the sender until it is held. Past [`KEY_WAIT_MS`] from composition it
//! fails with "the community key did not arrive" (Retry or Delete); our
//! departure discards it; a lock keeps it.
//!
//! Signal-Desktop keeps a send as a durable job for `MAX_RETRY_TIME =
//! durations.DAY` (`ts/jobs/conversationJobQueue.preload.ts`);
//! matrix-rust-sdk's send queue keeps a local echo `NotSentYet` and runs
//! key sharing as a step of the send; none refuses a send for a missing
//! group key, none falls back to plaintext.

use std::collections::HashSet;

use rekindle_types::message::SendState;

use rekindle_types::message::PostEntry;

use crate::deps::{ChannelMessagingDeps, ChannelPost, ChannelSendOutcome, SentChannelMessageEcho};
use crate::error::ChannelError;
use crate::pipeline::{random_message_id, ChannelSendResult};

/// How long a post waits for its channel's key from composition: one day
/// (Signal-Desktop's `MAX_RETRY_TIME = durations.DAY`, plan F9-1).
pub const KEY_WAIT_MS: i64 = 24 * 60 * 60 * 1000;

/// A post as the user composed it.
pub struct Composed<'a> {
    pub community_id: &'a str,
    pub channel_id: &'a str,
    pub body: &'a str,
    pub reply_to: Option<String>,
    pub sender_key: String,
    pub timestamp_ms: i64,
}

/// Whether a post to `channel_id` must wait: we hold no key to seal it, or
/// an earlier post of the channel still waits (first-composed first).
///
/// # Errors
/// `channel_id` is not a channel id.
pub async fn must_wait<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<bool, ChannelError> {
    if !crate::text_keys::holds_text_key(deps, community_id, channel_id)? {
        return Ok(true);
    }
    Ok(deps
        .waiting_posts(community_id)
        .await
        .iter()
        .any(|post| post.channel == channel_id))
}

/// Store `composed` waiting for its key: its row `awaiting_key` with its
/// outbox row, the echo, then a release in case the key is already here
/// (it waited only behind an earlier post).
///
/// # Errors
/// The store failed.
pub async fn queue_awaiting_key<D: ChannelMessagingDeps>(
    deps: &D,
    composed: Composed<'_>,
) -> Result<ChannelSendResult, ChannelError> {
    let Composed {
        community_id,
        channel_id,
        body,
        reply_to,
        sender_key,
        timestamp_ms,
    } = composed;
    let message_id = random_message_id("msg_");
    // The row's place is its stream seq, given when it is queued (D22).
    let outcome = ChannelSendOutcome {
        message_id: message_id.clone(),
        sender_pseudonym_hex: sender_key.clone(),
        ciphertext: Vec::new(),
        mek_generation: None,
        timestamp_ms,
        reply_to,
        send_state: SendState::AwaitingKey,
    };
    deps.persist_sent_message(community_id, channel_id, &outcome, body)
        .await?;
    deps.mark_last_send_at(community_id, channel_id, timestamp_ms);
    deps.persist_slowmode_state(community_id, channel_id, timestamp_ms)
        .await?;
    let post = ChannelPost {
        community: community_id.to_string(),
        channel: channel_id.to_string(),
        message: message_id.clone(),
        entry: PostEntry::Message,
    };
    let state = match deps
        .queue_waiting_post(&post, timestamp_ms.saturating_add(KEY_WAIT_MS))
        .await
    {
        Ok(()) => SendState::AwaitingKey,
        Err(error) => {
            tracing::warn!(%error, message = %message_id, "queueing the waiting post failed");
            SendState::Failed
        }
    };
    let timestamp_u64 = u64::try_from(timestamp_ms).unwrap_or_default();
    deps.emit_chat_event_local(&SentChannelMessageEcho {
        community_id: community_id.to_string(),
        message_id: message_id.clone(),
        sender_pseudonym: sender_key.clone(),
        timestamp_ms: timestamp_u64,
        body: body.to_string(),
        channel_id: channel_id.to_string(),
    });
    deps.emit_send_state(community_id, channel_id, &message_id, state);
    tracing::info!(message = %message_id, "channel post waits for the community key");
    if state == SendState::AwaitingKey
        && crate::text_keys::holds_text_key(deps, community_id, channel_id)?
    {
        release_waiting_posts(deps, community_id).await;
    }
    Ok(ChannelSendResult {
        status: state.as_str().to_string(),
        message_id,
        sender_pseudonym: sender_key,
        timestamp_ms: timestamp_u64,
        body: body.to_string(),
    })
}

/// A key of `community_id` was installed: release its waiting posts whose
/// channel's key we now hold to their stream (`queued`), each channel's in
/// composition order. A channel still without its key, or whose
/// head another release took, is left to that key's release.
pub async fn release_waiting_posts<D: ChannelMessagingDeps>(deps: &D, community_id: &str) {
    let mut held_back: HashSet<String> = HashSet::new();
    for post in deps.waiting_posts(community_id).await {
        if held_back.contains(&post.channel) {
            continue;
        }
        let keyed =
            crate::text_keys::holds_text_key(deps, community_id, &post.channel).unwrap_or(false);
        if !keyed || !deps.release_waiting_post(&post.message).await {
            held_back.insert(post.channel.clone());
            continue;
        }
        tracing::info!(message = %post.message, "channel post released: the community key came");
        deps.emit_send_state(
            &post.community,
            &post.channel,
            &post.message,
            SendState::Queued,
        );
    }
}
