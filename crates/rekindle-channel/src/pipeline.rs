//! Channel send and forward orchestrators (plan E3.3): a post is stored
//! `queued` in our `messages` row and queued on the channel's
//! `channel_post` outbox stream; the stream's sender seals it and appends
//! it to our member record's channel outbox (`M`/4,
//! [`crate::outbox_send::drain_posts`]), and the outbox moves its state.

use rekindle_types::message::{ChannelPost, PostEntry, SendState};
use rekindle_types::permissions::{BYPASS_SLOWMODE, SEND_MESSAGES};

use crate::deps::{ChannelMessagingDeps, ChannelSendOutcome, SentChannelMessageEcho};
use crate::error::ChannelError;
use crate::send::slowmode_check;

/// Architecture §28.7 — slowmode gate that combines the pure
/// `slowmode_check` decision with the `BYPASS_SLOWMODE` permission
/// shortcut. Adapter supplies the channel snapshot + permission bits.
pub fn enforce_slowmode_with_bypass<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) -> Result<(), ChannelError> {
    let Some(channel) = deps.channel_info(community_id, channel_id) else {
        return Ok(());
    };
    let Some(seconds) = channel.slowmode_seconds.filter(|&s| s > 0) else {
        return Ok(());
    };
    let perms = deps.compute_my_permissions(community_id);
    if perms & BYPASS_SLOWMODE == BYPASS_SLOWMODE {
        return Ok(());
    }
    let last_send = u64::try_from(channel.last_send_at_ms.unwrap_or(0)).unwrap_or(0);
    let now = u64::try_from(now_ms).unwrap_or(0);
    slowmode_check(Some(seconds), last_send, now).map_err(|e| match e {
        ChannelError::SlowmodeActive { wait_ms } => {
            let remaining_secs = wait_ms.div_ceil(1000);
            ChannelError::Adapter(format!(
                "slowmode active — wait {remaining_secs}s before sending again"
            ))
        }
        other => other,
    })
}

pub(crate) fn random_message_id(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        hex::encode(rekindle_utils::random::id_bytes_16())
    )
}

/// Send-result returned to the orchestrator caller.
#[derive(Debug, Clone)]
pub struct ChannelSendResult {
    pub status: String,
    pub message_id: String,
    pub sender_pseudonym: String,
    pub timestamp_ms: u64,
    pub body: String,
}

/// Send a channel post: refused in a forum channel, without
/// `SEND_MESSAGES` or under slowmode (with the `BYPASS_SLOWMODE`
/// shortcut); waits for the channel's key when we hold none
/// ([`crate::awaiting_key`]); else stored `queued` with its outbox row.
pub async fn send_channel_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    body: &str,
    reply_to: Option<String>,
) -> Result<ChannelSendResult, ChannelError> {
    let timestamp_ms = i64::try_from(deps.now_secs() * 1000).unwrap_or(i64::MAX);

    let info = deps
        .channel_info(community_id, channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    if info.is_forum {
        return Err(ChannelError::Adapter(
            "forum channels accept posts only through thread creation".into(),
        ));
    }
    deps.require_channel_permission(community_id, Some(channel_id), SEND_MESSAGES)?;
    enforce_slowmode_with_bypass(deps, community_id, channel_id, timestamp_ms)?;

    let sender_key = deps
        .my_pseudonym_hex(community_id)
        .or_else(|| deps.owner_key())
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    // No key for the channel yet, or an earlier post still waits for it:
    // queued until the key comes, never refused (plan E3.2 S7l, F9-1).
    if crate::awaiting_key::must_wait(deps, community_id, channel_id).await? {
        let composed = crate::awaiting_key::Composed {
            community_id,
            channel_id,
            body,
            reply_to,
            sender_key,
            timestamp_ms,
        };
        return crate::awaiting_key::queue_awaiting_key(deps, composed).await;
    }
    let message_id = random_message_id("msg_");
    // The row's place is its stream seq, given when it is queued (D22).
    let outcome = ChannelSendOutcome {
        message_id: message_id.clone(),
        sender_pseudonym_hex: sender_key.clone(),
        ciphertext: Vec::new(),
        mek_generation: None,
        timestamp_ms,
        reply_to,
        send_state: SendState::Queued,
    };
    deps.persist_sent_message(community_id, channel_id, &outcome, body)
        .await?;
    deps.mark_last_send_at(community_id, channel_id, timestamp_ms);
    deps.persist_slowmode_state(community_id, channel_id, timestamp_ms)
        .await?;
    let status = queue(
        deps,
        community_id,
        channel_id,
        &message_id,
        PostEntry::Message,
    )
    .await;

    let timestamp_u64 = u64::try_from(timestamp_ms).unwrap_or_default();
    deps.emit_chat_event_local(&SentChannelMessageEcho {
        community_id: community_id.to_string(),
        message_id: message_id.clone(),
        sender_pseudonym: sender_key.clone(),
        timestamp_ms: timestamp_u64,
        body: body.to_string(),
        channel_id: channel_id.to_string(),
        forwarded_from_author: None,
    });

    Ok(ChannelSendResult {
        status,
        message_id,
        sender_pseudonym: sender_key,
        timestamp_ms: timestamp_u64,
        body: body.to_string(),
    })
}

/// Queue our stored post `message_id` on its channel's stream; the state
/// name for the caller (`failed` when the row could not be stored).
async fn queue<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    entry: PostEntry,
) -> String {
    let post = ChannelPost {
        governance_cut: Vec::new(),
        community: community_id.to_string(),
        channel: channel_id.to_string(),
        message: message_id.to_string(),
        entry,
    };
    match deps.queue_post(&post).await {
        Ok(()) => SendState::Queued.as_str().to_string(),
        Err(error) => {
            tracing::warn!(%error, message = %post.message, "queueing the channel post failed");
            deps.emit_delivery_failed(community_id, channel_id, message_id);
            SendState::Failed.as_str().to_string()
        }
    }
}

/// Source/destination identifiers for a channel forward.
///
/// Borrows every id (`&'a str`) because the orchestrator already holds
/// these as owned strings; forwarding only reads them for cache lookup
/// and the destination write, so no clones are needed.
pub struct ForwardChannelMessageParams<'a> {
    pub source_community: &'a str,
    pub source_channel: &'a str,
    pub source_message: &'a str,
    pub dest_community: &'a str,
    pub dest_channel: &'a str,
}

/// Forward a cached source message into a destination channel: stored
/// `queued` with its outbox row, the source's body sealed when the stream
/// drains it. Cross-community-safe because pseudonyms aren't linkable
/// across community-scoped derivations (architecture §6.5).
pub async fn forward_channel_message<D: ChannelMessagingDeps>(
    deps: &D,
    params: ForwardChannelMessageParams<'_>,
) -> Result<ChannelSendResult, ChannelError> {
    let ForwardChannelMessageParams {
        source_community: _source_community_id,
        source_channel: source_channel_id,
        source_message: source_message_id,
        dest_community: dest_community_id,
        dest_channel: dest_channel_id,
    } = params;
    deps.require_channel_permission(dest_community_id, Some(dest_channel_id), SEND_MESSAGES)?;

    let dest_info = deps
        .channel_info(dest_community_id, dest_channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(dest_channel_id.into()))?;
    if dest_info.is_forum {
        return Err(ChannelError::Adapter(
            "forum channels accept posts only through thread creation".into(),
        ));
    }

    let timestamp_ms = i64::try_from(deps.now_secs() * 1000).unwrap_or(i64::MAX);
    enforce_slowmode_with_bypass(deps, dest_community_id, dest_channel_id, timestamp_ms)?;

    let source = deps
        .find_channel_message_by_id(source_channel_id, source_message_id)
        .await
        .ok_or_else(|| ChannelError::Adapter("source message not in local cache".into()))?;

    let forwarder_pseudonym = deps
        .my_pseudonym_hex(dest_community_id)
        .or_else(|| deps.owner_key())
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(dest_community_id.into()))?;
    let new_message_id = random_message_id("msg_");
    let outcome = ChannelSendOutcome {
        message_id: new_message_id.clone(),
        sender_pseudonym_hex: forwarder_pseudonym.clone(),
        ciphertext: Vec::new(),
        mek_generation: None,
        timestamp_ms,
        reply_to: None,
        send_state: SendState::Queued,
    };
    deps.persist_forwarded_message(
        dest_community_id,
        dest_channel_id,
        &outcome,
        &source.body,
        &source.sender_key,
    )
    .await?;
    deps.mark_last_send_at(dest_community_id, dest_channel_id, timestamp_ms);
    deps.persist_slowmode_state(dest_community_id, dest_channel_id, timestamp_ms)
        .await?;
    let entry = PostEntry::Forward {
        original_channel: source_channel_id.to_string(),
        original_message: source_message_id.to_string(),
        original_author: source.sender_key.clone(),
    };
    let status = queue(
        deps,
        dest_community_id,
        dest_channel_id,
        &new_message_id,
        entry,
    )
    .await;

    let timestamp_u64 = u64::try_from(timestamp_ms).unwrap_or_default();
    deps.emit_chat_event_local(&SentChannelMessageEcho {
        community_id: dest_community_id.to_string(),
        message_id: new_message_id.clone(),
        sender_pseudonym: forwarder_pseudonym.clone(),
        timestamp_ms: timestamp_u64,
        body: source.body.clone(),
        channel_id: dest_channel_id.to_string(),
        forwarded_from_author: Some(source.sender_key),
    });

    Ok(ChannelSendResult {
        status,
        message_id: new_message_id,
        sender_pseudonym: forwarder_pseudonym,
        timestamp_ms: timestamp_u64,
        body: source.body,
    })
}
