//! Our direct messages: send, history, read marks and the inbox.

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_types::dto::dm::{DmInboxEntry, DmMessage};
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::send::send_to_peer;
use crate::operations::friend::{state, FriendError, FriendHost};

/// Store our message to `to` and send it.
///
/// The send's outcome reaches the window: an error is returned (the
/// window marks the message failed, with Retry), and only a message that
/// went out or was queued is acknowledged (plan C7.28). E2.5's outbox
/// replaces the acknowledgement with its delivery states.
///
/// # Errors
/// The message could not be stored, or the send failed (it stays stored).
pub async fn send_dm(
    host: &dyn FriendHost,
    to: &str,
    body: &str,
) -> Result<DmMessage, FriendError> {
    state::require_public_key(to)?;
    let owner = state::owner(host)?;
    let timestamp = host.clock().now_ms_i64();
    tracing::info!(to = %to, len = body.len(), "sending message");
    let (ok, peer, text) = (owner.clone(), to.to_string(), body.to_string());
    let id = state::db_call!(host, "store sent message", move |conn| {
        rekindle_db::repo::messages::insert_dm(conn, &ok, &peer, &ok, &text, timestamp, true)
    })
    .await?;

    // Sending the message ends our typing to `to`.
    super::typing::message_sent(host, to);
    let payload = MessagePayload::DirectMessage {
        body: body.to_string(),
        reply_to: None,
    };
    if let Err(e) = send_to_peer(host, to, &payload).await {
        tracing::warn!(error = %e, "DM send failed — message persisted locally");
        return Err(e);
    }
    host.publish(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DirectMessageAcknowledged {
            message_id: timestamp.cast_unsigned(),
            conversation_id: to.to_string(),
        },
    ));
    Ok(DmMessage {
        id,
        sender_id: owner,
        body: body.to_string(),
        timestamp,
        is_own: true,
        // As the row stores it (the column default) until E2.5's outbox.
        send_state: rekindle_types::message::SendState::Delivered,
    })
}

/// The newest `limit` messages with `peer` before `before` (ms), oldest
/// first.
///
/// # Errors
/// The history could not be read.
pub async fn history(
    host: &dyn FriendHost,
    peer: &str,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<DmMessage>, FriendError> {
    let owner = state::owner(host)?;
    let (ok, pk) = (owner.clone(), peer.to_string());
    let rows = state::db_call!(host, "read message history", move |conn| {
        rekindle_db::repo::messages::dm_history(conn, &ok, &pk, limit, before)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DmMessage {
            id: row.id,
            is_own: row.sender_key == owner,
            sender_id: row.sender_key,
            body: row.body,
            timestamp: row.timestamp,
            send_state: row.send_state,
        })
        .collect())
}

/// Mark everything from `peer` read.
///
/// # Errors
/// The marks could not be stored.
pub async fn mark_read(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let pk = peer.to_string();
    state::db_call!(host, "mark messages read", move |conn| {
        rekindle_db::repo::messages::mark_dm_read(conn, &owner, &pk)
    })
    .await?;
    state::update(host, peer, |f| f.unread_count = 0);
    Ok(())
}

/// The latest message of each direct conversation, newest first.
///
/// # Errors
/// The inbox could not be read.
pub async fn inbox(host: &dyn FriendHost, limit: u32) -> Result<Vec<DmInboxEntry>, FriendError> {
    let owner = state::owner(host)?;
    let rows = state::db_call!(host, "read the inbox", move |conn| {
        rekindle_db::repo::messages::dm_inbox(conn, &owner, limit)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DmInboxEntry {
            peer_key: row.peer_key,
            sender_key: row.sender_key,
            body: row.body,
            timestamp: row.timestamp,
            is_read: row.is_read,
        })
        .collect())
}
