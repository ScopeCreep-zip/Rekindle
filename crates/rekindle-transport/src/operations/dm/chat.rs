//! Our direct messages: send, history, read marks and the inbox.

use rekindle_codec::capnp_codec::direct::{ClientId, DirectBody, DmText};
use rekindle_types::dto::dm::{DmInboxEntry, DmMessage};
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::send::send_to_peer;
use crate::operations::friend::{state, FriendError, FriendHost};

/// A fresh client id: a UUID v7 from the Host's clock and generator
/// (C9.2), so ids sort by send time.
pub(crate) fn mint_client_id(host: &dyn FriendHost) -> ClientId {
    let mut random = [0u8; 10];
    host.rng().fill(&mut random);
    *uuid::Builder::from_unix_timestamp_millis(host.clock().now_ms(), &random)
        .into_uuid()
        .as_bytes()
}

/// A client id as the `messages.message_id` column stores it.
#[must_use]
pub fn client_id_text(id: &ClientId) -> String {
    uuid::Uuid::from_bytes(*id).to_string()
}

/// The client id in `text` (a UUID), as a reply names its parent.
///
/// # Errors
/// `Invalid` when `text` is not a UUID.
pub fn parse_client_id(text: &str) -> Result<ClientId, FriendError> {
    uuid::Uuid::parse_str(text)
        .map(|id| *id.as_bytes())
        .map_err(|e| FriendError::Invalid(format!("message id {text}: {e}")))
}

/// A direct message to store, owned so it crosses to the database thread
/// (`rekindle_db::repo::messages::NewDm`).
pub(crate) struct NewDmFields {
    pub peer: String,
    pub sender: String,
    pub body: String,
    pub timestamp: i64,
    pub is_read: bool,
    pub message_id: String,
    pub reply_to: Option<String>,
}

impl NewDmFields {
    /// Store it; its row, and whether it was new (a replay finds it held).
    ///
    /// # Errors
    /// The identity is locked, or the write failed.
    pub(crate) async fn store(
        self,
        host: &dyn FriendHost,
    ) -> Result<rekindle_db::repo::messages::StoredDm, FriendError> {
        self.store_with(host, None).await
    }

    /// Store it and acknowledge the peer's verified key change in the
    /// same transaction: the send a `VerifiedKeyChanged` review held was
    /// committed (plan E2.3).
    ///
    /// # Errors
    /// The identity is locked, a key is malformed, or the write failed
    /// (neither is kept).
    async fn store_acknowledging(
        self,
        host: &dyn FriendHost,
    ) -> Result<rekindle_db::repo::messages::StoredDm, FriendError> {
        let keys = crate::operations::friend::verification::keys(host, &self.peer)?;
        self.store_with(host, Some(keys)).await
    }

    async fn store_with(
        self,
        host: &dyn FriendHost,
        acknowledge: Option<([u8; 32], [u8; 32])>,
    ) -> Result<rekindle_db::repo::messages::StoredDm, FriendError> {
        let owner = state::owner(host)?;
        state::db_call!(host, "store direct message", move |conn| {
            let dm = rekindle_db::repo::messages::NewDm {
                owner_key: &owner,
                peer_key: &self.peer,
                sender_key: &self.sender,
                body: &self.body,
                timestamp: self.timestamp,
                is_read: self.is_read,
                message_id: &self.message_id,
                reply_to: self.reply_to.as_deref(),
            };
            match acknowledge {
                Some((ours, theirs)) => rekindle_db::repo::signal_identities::acknowledging(
                    conn,
                    &ours,
                    &theirs,
                    |tx| rekindle_db::repo::messages::insert_dm(tx, &dm),
                ),
                None => rekindle_db::repo::messages::insert_dm(conn, &dm),
            }
        })
        .await
    }
}

/// Store our message to `to` (a reply to `reply_to`, the parent's client
/// id, when given) and send it. A send whose review was held on `to`'s
/// verified key changing acknowledges the change with its own row
/// (`acknowledges_key_change`, plan E2.3).
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
    reply_to: Option<&str>,
    acknowledges_key_change: bool,
) -> Result<DmMessage, FriendError> {
    state::require_public_key(to)?;
    let reply_to = reply_to.map(parse_client_id).transpose()?;
    let owner = state::owner(host)?;
    let timestamp = host.clock().now_ms_i64();
    let client_id = mint_client_id(host);
    let message_id = client_id_text(&client_id);
    tracing::info!(to = %to, len = body.len(), "sending message");
    let fields = NewDmFields {
        peer: to.to_string(),
        sender: owner.clone(),
        body: body.to_string(),
        timestamp,
        is_read: true,
        message_id: message_id.clone(),
        reply_to: reply_to.as_ref().map(client_id_text),
    };
    let stored = if acknowledges_key_change {
        fields.store_acknowledging(host).await?
    } else {
        fields.store(host).await?
    };

    let send_state = deliver(host, &owner, to, client_id, body, reply_to).await?;
    Ok(DmMessage {
        id: stored.id,
        message_id: Some(message_id),
        sender_id: owner,
        body: body.to_string(),
        timestamp,
        is_own: true,
        send_state,
    })
}

/// Frame our message `client_id` to `to` and send it, store its send state
/// and tell the frontends (`DirectMessageStateChanged`); the state, or the
/// send's error once the failure is recorded.
async fn deliver(
    host: &dyn FriendHost,
    owner: &str,
    to: &str,
    client_id: ClientId,
    body: &str,
    reply_to: Option<ClientId>,
) -> Result<rekindle_types::message::SendState, FriendError> {
    // Sending the message ends our typing to `to`.
    super::typing::message_sent(host, to);
    let message = DirectBody::DmText(DmText {
        client_id,
        body: body.to_string(),
        reply_to,
        expire_seconds: 0,
    });
    let sent = send_to_peer(host, to, &message).await;
    let send_state = if sent.is_ok() {
        rekindle_types::message::SendState::Sent
    } else {
        rekindle_types::message::SendState::Failed
    };
    let message_id = client_id_text(&client_id);
    let (ok, mid) = (owner.to_string(), message_id.clone());
    if let Err(e) = state::db_call!(host, "store the send state", move |conn| {
        rekindle_db::repo::messages::set_send_state(conn, &ok, &mid, send_state, None)
    })
    .await
    {
        tracing::warn!(error = %e, "send state not stored");
    }
    host.publish(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DirectMessageStateChanged {
            message_id,
            conversation_id: to.to_string(),
            send_state,
        },
    ));
    if let Err(e) = sent {
        tracing::warn!(error = %e, "DM send failed — message persisted locally");
        return Err(e);
    }
    Ok(send_state)
}

/// The peer of our stored message `message_id` (a client id), for the
/// resend's review; `None` when we sent none with that id.
///
/// # Errors
/// The identity is locked or the read failed.
pub async fn resend_peer(
    host: &dyn FriendHost,
    message_id: &str,
) -> Result<Option<String>, FriendError> {
    Ok(own_dm(host, message_id).await?.map(|dm| dm.row.peer_key))
}

async fn own_dm(
    host: &dyn FriendHost,
    message_id: &str,
) -> Result<Option<rekindle_db::repo::messages::OwnDm>, FriendError> {
    let owner = state::owner(host)?;
    let mid = message_id.to_string();
    state::db_call!(host, "read the message to resend", move |conn| {
        rekindle_db::repo::messages::own_dm(conn, &owner, &mid)
    })
    .await
}

/// Send our failed message `message_id` (its client id) again: the stored
/// row is framed under the same client id, so the peer keeps one copy, and
/// no new row is stored. A resend whose review was held on the peer's
/// verified key changing acknowledges the change (plan E2.3).
///
/// # Errors
/// `NotFound` when we sent no message with that id, `Conflict` when it did
/// not fail, or the send failed (it stays failed).
pub async fn resend_dm(
    host: &dyn FriendHost,
    message_id: &str,
    acknowledges_key_change: bool,
) -> Result<DmMessage, FriendError> {
    let client_id = parse_client_id(message_id)?;
    let reply_to;
    let row = {
        let dm = own_dm(host, message_id)
            .await?
            .ok_or_else(|| FriendError::NotFound(format!("message {message_id}")))?;
        reply_to = dm.reply_to.as_deref().map(parse_client_id).transpose()?;
        dm.row
    };
    if row.send_state != rekindle_types::message::SendState::Failed {
        return Err(FriendError::Conflict(format!(
            "message {message_id} did not fail"
        )));
    }
    let owner = state::owner(host)?;
    if acknowledges_key_change {
        let (ours, theirs) = crate::operations::friend::verification::keys(host, &row.peer_key)?;
        state::db_call!(host, "acknowledge the key change", move |conn| {
            rekindle_db::repo::signal_identities::acknowledging(conn, &ours, &theirs, |_| Ok(()))
        })
        .await?;
    }
    tracing::info!(to = %row.peer_key, len = row.body.len(), "resending message");
    let send_state = deliver(host, &owner, &row.peer_key, client_id, &row.body, reply_to).await?;
    Ok(DmMessage {
        id: row.id,
        message_id: Some(message_id.to_string()),
        sender_id: owner,
        body: row.body,
        timestamp: row.timestamp,
        is_own: true,
        send_state,
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
            message_id: row.message_id,
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
    publish_unread(host, peer, 0);
    Ok(())
}

/// Count one more unread message from `peer` and tell the frontends the
/// count, which they render as is.
pub(crate) fn count_unread(host: &dyn FriendHost, peer: &str) {
    let mut count = 0;
    state::update(host, peer, |f| {
        f.unread_count += 1;
        count = f.unread_count;
    });
    publish_unread(host, peer, count);
}

fn publish_unread(host: &dyn FriendHost, peer: &str, count: u32) {
    host.publish(SubscriptionEvent::UnreadChanged {
        context: rekindle_types::subscription_events::UnreadContext::Dm {
            peer_key: peer.to_string(),
        },
        count,
    });
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
