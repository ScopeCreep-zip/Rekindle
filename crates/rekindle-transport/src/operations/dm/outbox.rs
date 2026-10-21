//! The DM stream on the outbox (plan E2.5, D22), as the transport sees it.
//! A direct message is committed with its local echo as one row of the
//! `dm:<peer>` stream and framed at each attempt, so the ratchet message
//! carries the entry's stream seq that the peer's `DmAck` names. The
//! engine and its storage are the Host's (`rekindle-outbox`, behind
//! [`DmOutbox`]); this module only frames and delivers.

use async_trait::async_trait;
use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};
use serde::{Deserialize, Serialize};

use super::send;
use crate::operations::friend::{FriendError, FriendHost};

/// Our message to commit with its local echo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingDm {
    /// The id every attempt carries (`messages.message_id`).
    pub client_id: String,
    /// [`payload`] of the message's body.
    pub payload: Vec<u8>,
    /// Our identity key, the echo's sender.
    pub sender: String,
    pub body: String,
    pub timestamp: i64,
    pub reply_to: Option<String>,
}

/// The Host's outbox, as the 1:1 operations use it.
#[async_trait]
pub trait DmOutbox: Send + Sync {
    /// Commit `dm` with its echo at the end of our stream to `to`, in one
    /// transaction; its stream seq.
    ///
    /// # Errors
    /// The storage failed; nothing was committed.
    async fn enqueue(&self, to: &str, dm: OutgoingDm) -> Result<i64, FriendError>;
    /// Commit `dm` with its echo (a `group_dm` message under the group's
    /// id) at the end of our stream to `group_id` (plan E2.6), in one
    /// transaction; its stream seq.
    ///
    /// # Errors
    /// The storage failed; nothing was committed.
    async fn enqueue_group(&self, group_id: &str, dm: OutgoingDm) -> Result<i64, FriendError>;
    /// Commit a `friend_handshake` frame (`payload`, [`super::Framed`]) at
    /// the end of our handshake stream to `to`; its stream seq.
    ///
    /// # Errors
    /// The storage failed; nothing was committed.
    /// Commit a control frame (`payload`, [`payload`]) with no echo at
    /// the end of our `dm` stream to `to` (a `StreamRecord`): it waits for
    /// the peer's `DmAck` like a message; its stream seq.
    ///
    /// # Errors
    /// The storage failed; nothing was committed.
    async fn enqueue_control(
        &self,
        to: &str,
        client_id: String,
        payload: Vec<u8>,
    ) -> Result<i64, FriendError>;
    async fn enqueue_handshake(
        &self,
        to: &str,
        client_id: String,
        payload: Vec<u8>,
    ) -> Result<i64, FriendError>;
    /// Mark `to` removing and commit our `Unfriended` frame to them, in one
    /// transaction (plan audit D-1).
    ///
    /// # Errors
    /// The storage failed; nothing was committed.
    async fn enqueue_removal(
        &self,
        to: &str,
        client_id: String,
        payload: Vec<u8>,
    ) -> Result<(), FriendError>;
    /// The peer acknowledged row `client_id` by a frame naming no stream
    /// seq (`UnfriendedAck`); `false` when no row waits.
    ///
    /// # Errors
    /// The storage failed.
    async fn acknowledge_client(&self, client_id: &str) -> Result<bool, FriendError>;
    /// The user's Discard: the row settles `failed` and leaves its stream.
    ///
    /// # Errors
    /// The storage failed.
    async fn discard(&self, client_id: &str) -> Result<bool, FriendError>;
    /// The user's Retry under the same client id; `false` when no row waits.
    ///
    /// # Errors
    /// The storage failed.
    async fn retry(&self, client_id: &str) -> Result<bool, FriendError>;
    /// `from` holds entry `stream_seq` of our stream to them (`DmAck`).
    ///
    /// # Errors
    /// The storage failed.
    async fn acknowledge(&self, from: &str, stream_seq: i64) -> Result<bool, FriendError>;
    /// `from` read our stream to them through `stream_seq` (`ReadUpTo`):
    /// every entry at or below it becomes `read`; how many changed.
    ///
    /// # Errors
    /// The storage failed.
    async fn read_up_to(&self, from: &str, stream_seq: i64) -> Result<usize, FriendError>;
    /// Entry `client_id` of `from`'s stream was committed here: `true` the
    /// first time, `false` for a resend already applied.
    ///
    /// # Errors
    /// The storage failed.
    async fn receive(&self, from: &str, client_id: &str) -> Result<bool, FriendError>;
}

/// A `dm` row's payload: the body, framed afresh at each attempt.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    type_id: u16,
    #[serde(with = "rekindle_types::base64_bytes")]
    body: Vec<u8>,
}

/// `body` as a `dm` row stores it.
///
/// # Errors
/// The payload could not be serialized.
pub fn payload(body: &DirectBody) -> Result<Vec<u8>, FriendError> {
    serde_json::to_vec(&Payload {
        type_id: body.type_id().as_u16(),
        body: body.encode(),
    })
    .map_err(|e| FriendError::Failed(format!("serialize the outbox payload: {e}")))
}

/// A `dm` or `group_dm` row's payload read back.
///
/// # Errors
/// It is not a stored payload of an allocated kind.
pub(crate) fn decode_payload(payload: &[u8]) -> Result<DirectBody, String> {
    decode(payload)
}

fn decode(payload: &[u8]) -> Result<DirectBody, String> {
    let stored: Payload = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
    let type_id = DmTypeId::from_u16(stored.type_id)
        .ok_or_else(|| format!("typeId {:#06x}", stored.type_id))?;
    DirectBody::decode(type_id, &stored.body).map_err(|e| e.to_string())
}

/// How one attempt of a `dm` row went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    /// The frame left this device.
    Sent,
    /// The frame left this device, and the row waits for the peer's ack
    /// of it (`Unfriended`).
    Awaiting,
    /// The peer's `app_call` reply received it (a `FriendRequest`).
    Receipted,
    /// Not now: no route, no session yet, the node is not up, or the
    /// peer's changed key waits for the user's review.
    Deferred,
    /// It can never be sent as it stands.
    Wedged(String),
}

/// Frame entry `stream_seq` of our stream to `to`, append it to our chain
/// to them (plan E2.5, D23: the chain is authoritative), then send it once
/// as the hint that makes the peer diff. Once on the chain it counts as
/// sent whether or not the hint left.
pub async fn attempt(host: &dyn FriendHost, to: &str, stream_seq: i64, payload: &[u8]) -> Attempt {
    let body = match decode(payload) {
        Ok(body) => body,
        Err(e) => return Attempt::Wedged(format!("payload: {e}")),
    };
    let seq = u64::try_from(stream_seq).unwrap_or(0);
    let frame = match send::stream_frame(host, to, &body, seq).await {
        Ok(frame) => frame,
        Err(FriendError::Locked | FriendError::NotReady(_) | FriendError::Conflict(_)) => {
            return Attempt::Deferred;
        }
        Err(e) => return Attempt::Wedged(e.to_string()),
    };
    let datagram = match super::frame::wire(&frame) {
        Ok(datagram) => datagram,
        Err(e) => return Attempt::Wedged(e.to_string()),
    };
    // A `StreamRecord` names the chain; it is not on it, and it waits for
    // the peer's `DmAck` (resent until then) like any `dm` row.
    if matches!(body, DirectBody::StreamRecord(_)) {
        return match send::route_send_datagram(host, to, &datagram, frame.type_id, true).await {
            Ok(true) => Attempt::Sent,
            Ok(false) | Err(FriendError::NotReady(_)) => Attempt::Deferred,
            Err(e) => Attempt::Wedged(e.to_string()),
        };
    }
    match super::stream::append(host, to, &datagram, stream_seq).await {
        Ok(_) => {}
        Err(FriendError::Locked | FriendError::NotReady(_) | FriendError::Conflict(_)) => {
            return Attempt::Deferred;
        }
        Err(e) => return Attempt::Wedged(e.to_string()),
    }
    if let Err(e) = send::route_send_datagram(host, to, &datagram, frame.type_id, true).await {
        tracing::debug!(to = %to, error = %e, "chain entry hint not sent");
    }
    Attempt::Sent
}

#[cfg(test)]
mod tests {
    use rekindle_codec::capnp_codec::direct::DmText;

    use super::*;

    #[test]
    fn a_payload_round_trips_its_body() {
        let body = DirectBody::DmText(DmText {
            client_id: [7; 16],
            body: "hi".into(),
            reply_to: None,
            expire_seconds: 0,
        });
        assert_eq!(decode(&payload(&body).unwrap()).unwrap(), body);
        assert!(decode(b"{}").is_err());
    }
}
