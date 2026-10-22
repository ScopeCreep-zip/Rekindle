//! Our direct messages: send, history, read marks and the inbox.

use rekindle_codec::capnp_codec::direct::{ClientId, DirectBody, DmText};
use rekindle_types::dto::dm::{DmInboxEntry, DmMessage};
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::SubscriptionEvent;

use super::outbox::OutgoingDm;
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
    /// `Delivered` for a received one (plan audit D-3); ours is the
    /// outbox's echo.
    pub send_state: SendState,
    /// The entry's place in the sender's DM stream (plan E2.5).
    pub stream_seq: Option<i64>,
}

impl NewDmFields {
    /// Store a received one at its clamped time
    /// (`rekindle_db::repo::received_dm`); its row and the time stored.
    ///
    /// # Errors
    /// The identity is locked, or the write failed.
    pub(crate) async fn store_received(
        self,
        host: &dyn FriendHost,
    ) -> Result<(rekindle_db::repo::messages::StoredDm, i64), FriendError> {
        let owner = state::owner(host)?;
        let received_at = host.clock().now_ms_i64();
        state::db_call!(host, "store direct message", move |conn| {
            let dm = self.new_dm(&owner);
            rekindle_db::repo::received_dm::insert_received(
                conn,
                &dm,
                received_at,
                super::MAX_FUTURE_SKEW_MS,
            )
        })
        .await
    }

    fn new_dm<'a>(&'a self, owner: &'a str) -> rekindle_db::repo::messages::NewDm<'a> {
        rekindle_db::repo::messages::NewDm {
            owner_key: owner,
            peer_key: &self.peer,
            conversation_type: rekindle_types::message::ConversationType::Dm,
            sender_key: &self.sender,
            body: &self.body,
            timestamp: self.timestamp,
            is_read: self.is_read,
            message_id: &self.message_id,
            reply_to: self.reply_to.as_deref(),
            send_state: self.send_state,
            stream_seq: self.stream_seq,
        }
    }
}

/// Acknowledge `peer`'s verified key change: the send its review held was
/// committed (plan E2.3).
async fn acknowledge_key_change(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    let (ours, theirs) = crate::operations::friend::verification::keys(host, peer)?;
    state::db_call!(host, "acknowledge the key change", move |conn| {
        rekindle_db::repo::signal_identities::acknowledging(conn, &ours, &theirs, |_| Ok(()))
    })
    .await
}

/// Commit our message to `to` (a reply to `reply_to`, the parent's client
/// id, when given) to the outbox with its local echo, in one transaction,
/// and return once it is committed (plan E2.5, D22): `queued`, then the
/// outbox's `DmSendStatus` states. A send whose review was held on `to`'s
/// verified key changing acknowledges the change first
/// (`acknowledges_key_change`, plan E2.3).
///
/// # Errors
/// The message could not be committed.
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
    let outbox = host.outbox()?;
    let timestamp = host.clock().now_ms_i64();
    let client_id = mint_client_id(host);
    let message_id = client_id_text(&client_id);
    if acknowledges_key_change {
        acknowledge_key_change(host, to).await?;
    }
    let payload = super::outbox::payload(&DirectBody::DmText(DmText {
        client_id,
        body: body.to_string(),
        reply_to,
        expire_seconds: 0,
    }))?;
    tracing::debug!(to = %to, "committing message to the outbox");
    outbox
        .enqueue(
            to,
            OutgoingDm {
                client_id: message_id.clone(),
                payload,
                sender: owner.clone(),
                body: body.to_string(),
                timestamp,
                reply_to: reply_to.as_ref().map(client_id_text),
            },
        )
        .await?;
    // Sending the message ends our typing to `to`.
    super::typing::message_sent(host, to);
    // Its stored time: never behind our earlier messages to `to`.
    let row = own_dm(host, &message_id)
        .await?
        .map(|dm| dm.row)
        .ok_or_else(|| FriendError::Failed(format!("message {message_id} not committed")))?;
    Ok(DmMessage {
        id: row.id,
        message_id: Some(message_id),
        sender_id: owner,
        body: body.to_string(),
        timestamp: row.timestamp,
        is_own: true,
        send_state: SendState::Queued,
        history_gap_through: None,
    })
}

/// The peer of our stored message `message_id` (a client id), for the
/// resend's review; `None` when we sent none with that id.
///
/// # Errors
/// The identity is locked or the read failed.
pub async fn retry_peer(
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

/// The user's Retry of any outbox row by its client id (plan E2.5): a
/// direct message, a handshake row (a request, an invite's redemption) or
/// a channel post, queued again under the **same** client id, so the peer
/// keeps one copy, and no new row is stored; a channel post's write is
/// made again through the channel send path (`OutboxDeps::reissue`). A
/// direct message must have failed; a retry whose review was held on the
/// peer's verified key changing acknowledges the change (plan E2.3, R-4).
///
/// # Errors
/// `Conflict` when our direct message with that id did not fail,
/// `NotFound` when no outbox row has the id, or the storage failed.
pub async fn retry_send(
    host: &dyn FriendHost,
    client_id: &str,
    acknowledges_key_change: bool,
) -> Result<(), FriendError> {
    let outbox = host.outbox()?;
    if let Some(dm) = own_dm(host, client_id).await? {
        if dm.row.send_state != SendState::Failed {
            return Err(FriendError::Conflict(format!(
                "message {client_id} did not fail"
            )));
        }
        if acknowledges_key_change {
            acknowledge_key_change(host, &dm.row.peer_key).await?;
        }
    }
    if !outbox.retry(client_id).await? {
        return Err(FriendError::NotFound(format!("send {client_id}")));
    }
    tracing::debug!(%client_id, "send retried by the user");
    Ok(())
}

/// The user's Discard of an outbox row by its client id: it settles
/// `failed` and leaves its stream, so the rows behind it go on.
///
/// # Errors
/// `NotFound` when no outbox row has the id, or the storage failed.
pub async fn discard_send(host: &dyn FriendHost, client_id: &str) -> Result<(), FriendError> {
    if !host.outbox()?.discard(client_id).await? {
        return Err(FriendError::NotFound(format!("send {client_id}")));
    }
    Ok(())
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
            history_gap_through: row.history_gap_through,
        })
        .collect())
}

/// Mark `peer`'s messages read up to `up_to`, the client id of the newest
/// message of theirs the focused window rendered (all of them when `None`).
/// The DM read marker (plan E3.1, D25) advances to that message's
/// `stream_seq`, the position `ReadUpTo` names; it never moves back. Our
/// `ReadUpTo` for it goes to them when our receipts setting is on (plan
/// E2.5, [`super::receipts`]), and the count derived from the marker is
/// published.
///
/// # Errors
/// The marker could not be stored.
pub async fn mark_read(
    host: &dyn FriendHost,
    peer: &str,
    up_to: Option<&str>,
) -> Result<(), FriendError> {
    use rekindle_db::repo::{messages, read_markers};
    let owner = state::owner(host)?;
    let (pk, through) = (peer.to_string(), up_to.map(str::to_owned));
    let now = host.clock().now_ms_i64();
    let count = state::db_call!(host, "advance the DM read marker", move |conn| {
        messages::mark_dm_read(conn, &owner, &pk)?;
        let position = match through {
            Some(message_id) => messages::received_stream_seq(conn, &owner, &pk, &message_id)?
                .and_then(|seq| u64::try_from(seq).ok()),
            None => Some(read_markers::dm_latest(conn, &owner, &pk)?),
        };
        let scope = read_markers::Scope::Dm {
            peer_key: pk.clone(),
        };
        if let Some(seq) = position {
            read_markers::advance(conn, &owner, &scope, seq, now)?;
        }
        let after = read_markers::get(conn, &owner, &scope)?;
        read_markers::dm_unread(conn, &owner, &pk, after)
    })
    .await?;
    publish_unread(host, peer, count);
    if let Some(message_id) = up_to {
        super::receipts::rendered(host, peer, message_id).await?;
    }
    Ok(())
}

/// A message from `peer` was stored: publish the count derived from the DM
/// read marker, which the frontends render as is.
pub(crate) async fn publish_dm_unread(host: &dyn FriendHost, peer: &str) {
    use rekindle_db::repo::read_markers;
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let pk = peer.to_string();
    let count = state::db_call!(host, "count the unread DMs", move |conn| {
        let scope = read_markers::Scope::Dm {
            peer_key: pk.clone(),
        };
        let after = read_markers::get(conn, &owner, &scope)?;
        read_markers::dm_unread(conn, &owner, &pk, after)
    })
    .await;
    match count {
        Ok(count) => publish_unread(host, peer, count),
        Err(e) => tracing::warn!(peer = %peer, error = %e, "DM unread count not published"),
    }
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
