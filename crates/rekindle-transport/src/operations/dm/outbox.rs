//! The DM stream on the outbox (plan E2.5, D22), as the transport sees it.
//! A direct message is committed with its local echo as one row of the
//! `dm:<peer>` stream and framed at each attempt, so the ratchet message
//! carries the entry's stream seq that the peer's `DmAck` names. The
//! engine and its storage are the Host's (`rekindle-outbox`, behind
//! [`DmOutbox`]); this module only frames and delivers.

use async_trait::async_trait;
use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};
use serde::{Deserialize, Serialize};

use rekindle_types::message::SendState;

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
    /// The friendship (or the attempt at one) with `peer` ended: every
    /// unsettled row of our `dm` stream to them settles `failed` with
    /// `reason` and leaves it (live A/B findings 31, 35); how many.
    ///
    /// # Errors
    /// The storage failed.
    async fn discard_to(&self, peer: &str, reason: &str) -> Result<usize, FriendError>;
    /// A new friendship with `peer` starts: every unsettled row of our
    /// `friend_handshake` stream to them, framed under the friendship that
    /// ended (an `Unfriended` awaiting its ack, an old `FriendsKey`),
    /// settles `failed` with `reason` and leaves it (live A/B finding 38);
    /// how many.
    ///
    /// # Errors
    /// The storage failed.
    async fn discard_handshakes_to(&self, peer: &str, reason: &str) -> Result<usize, FriendError>;
    /// `peer`'s answer proves our handshake rows to them whose client ids
    /// start with `client_prefix` arrived (their `FriendAccept` answers our
    /// `FriendRequest`, whose receipts may all have been lost): each
    /// unsettled one settles `delivered`, as its receipt would have, so
    /// the rows behind it go (live A/B findings 56, 60); how many.
    ///
    /// # Errors
    /// The storage failed.
    async fn receipt_handshakes_to(
        &self,
        peer: &str,
        client_prefix: &str,
    ) -> Result<usize, FriendError>;
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
    /// `from` applied our chain to them through chain seq `through` (a
    /// cumulative `DmAck`): every entry it carried is delivered, row or
    /// none; how many changed.
    ///
    /// # Errors
    /// The storage failed.
    async fn acknowledge_chain_through(
        &self,
        from: &str,
        through: u64,
    ) -> Result<usize, FriendError>;
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
    /// The hint left, and the entry (this frame) goes to the stream's
    /// appender; the append's outcome is its landing (the DM pump).
    Appending(Vec<u8>),
    /// Not now: no route, no session yet, the node is not up, or the
    /// peer's changed key waits for the user's review.
    Deferred,
    /// It can never be sent as it stands.
    Wedged(String),
    /// It is no longer worth sending (a first contact the peer refused,
    /// whose `PreKeyRejected` sends the retry): it settles failed and
    /// leaves its stream.
    Superseded(String),
}

/// One `dm` row of our stream to a peer, as [`attempt`] takes it.
#[derive(Debug, Clone, Copy)]
pub struct RowAttempt<'a> {
    pub stream_seq: i64,
    /// The row's state: `sent` for a chain entry means its append landed,
    /// and a later attempt does not append it again (live A/B finding 42);
    /// it is hinted again, on the dm backoff, until the peer's ack (live
    /// A/B finding 64).
    pub state: SendState,
    /// When the entry was written (its echo's time; now for a row with
    /// none).
    pub sent_at: Option<i64>,
    pub payload: &'a [u8],
}

/// A row framed for the wire, or how its attempt already ended.
enum Framed {
    /// A `StreamRecord`: it names the chain and is not on it.
    Record(Vec<u8>, DmTypeId),
    /// A frame for the chain.
    Entry(Vec<u8>, DmTypeId),
}

/// Frame `rows` (the due run of our stream to `to`, in `stream_seq`
/// order), append the frames to our chain to them at once (plan E2.5,
/// D23: the chain is authoritative; the run fills as few pages as hold it,
/// E2.B F2) while each is sent once as the hint that makes the peer diff.
/// Once on the chain a row counts as sent whether or not its hint left; a
/// row already on it ([`landed`]) is not appended again, only hinted again
/// until its ack ([`rehint`]).
/// Every attempt's frame carries `sent_at`, so the peer orders it by that
/// and not by the attempt that reached them (E2.B F3). The outcomes of a
/// prefix of `rows`, in order: the rows after the first that stays behind
/// (`rekindle_outbox::holds_back`) are not attempted.
pub async fn attempt(host: &dyn FriendHost, to: &str, rows: &[RowAttempt<'_>]) -> Vec<Attempt> {
    let mut outcomes: Vec<Option<Attempt>> = Vec::with_capacity(rows.len());
    let mut chained: Vec<(usize, Vec<u8>, DmTypeId)> = Vec::new();
    for row in rows {
        if landed(row) {
            rehint(host, to, row).await;
            outcomes.push(Some(Attempt::Sent));
            continue;
        }
        let outcome = match frame_row(host, to, row).await {
            Err(outcome) => outcome,
            // A `StreamRecord` waits for the peer's `DmAck` (resent until
            // then) like any `dm` row.
            Ok(Framed::Record(datagram, type_id)) => {
                match send::route_send_datagram(host, to, &datagram, type_id, true).await {
                    Ok(true) => Attempt::Sent,
                    Ok(false) => {
                        tracing::debug!(to = %to, ?type_id, "dm row deferred: no usable route");
                        Attempt::Deferred
                    }
                    Err(e @ FriendError::NotReady(_)) => attempt_of(e),
                    Err(e) => Attempt::Wedged(e.to_string()),
                }
            }
            Ok(Framed::Entry(datagram, type_id)) => {
                chained.push((outcomes.len(), datagram, type_id));
                outcomes.push(None);
                continue;
            }
        };
        if push_stops(&mut outcomes, outcome) {
            break;
        }
    }
    // The run's hints go at once, in order, and its appends to the
    // stream's appender ([`append`]), so the peer has each message without
    // waiting on a DHT write, its own or an earlier one's (live A/B finding
    // 42, the DM pump); an ack that beats the landing waits on the row.
    for (i, datagram, type_id) in chained {
        if let Err(e) = send::route_send_datagram(host, to, &datagram, type_id, true).await {
            tracing::debug!(to = %to, error = %e, "chain entry hint not sent");
        }
        outcomes[i] = Some(Attempt::Appending(datagram));
    }
    finish(outcomes)
}

/// Hint `row` again, a chain entry whose append landed and whose `DmAck`
/// has not come (live A/B finding 64): the peer's acks are never queued
/// (`dm::ack`), so a lost one is asked for again only by this resend. It is
/// framed afresh (the landed frame again would be dropped as a duplicate
/// before the peer applies and acks it, `frame::open`) under the same
/// stream seq and `sent_at`, and is not appended again: the entry is on the
/// chain. An acked row is gone from the outbox (`Outbox::acknowledge`), so
/// it is never hinted again. It stays sent whatever the hint does.
async fn rehint(host: &dyn FriendHost, to: &str, row: &RowAttempt<'_>) {
    match frame_row(host, to, row).await {
        Ok(Framed::Entry(datagram, type_id) | Framed::Record(datagram, type_id)) => {
            if let Err(e) = send::route_send_datagram(host, to, &datagram, type_id, true).await {
                tracing::debug!(to = %to, error = %e, "landed entry re-hint not sent");
            }
        }
        Err(outcome) => {
            tracing::debug!(to = %to, ?outcome, "landed entry re-hint not framed");
        }
    }
}

/// Append `frames` (handed-off entries, each with its `stream_seq`, in
/// order) to our chain to `to`: `Sent` for each landed, then the error that
/// stopped the rest; the rows after it were not appended.
pub async fn append(host: &dyn FriendHost, to: &str, frames: &[(Vec<u8>, i64)]) -> Vec<Attempt> {
    let (landed, appended) = super::stream::append(host, to, frames).await;
    landing(frames.len(), landed, appended)
}

/// The outcomes of a run append of `n` entries that landed `landed`.
pub(crate) fn landing(n: usize, landed: usize, appended: Result<(), FriendError>) -> Vec<Attempt> {
    let mut outcomes = vec![None; n];
    let slots: Vec<usize> = (0..n).collect();
    settle_chained(&mut outcomes, &slots, landed, appended);
    finish(outcomes)
}

/// Whether `row` is a chain entry whose append already landed: a
/// `StreamRecord` is not on the chain and is sent again until its ack.
pub(crate) fn landed(row: &RowAttempt<'_>) -> bool {
    row.state == SendState::Sent && !matches!(decode(row.payload), Ok(DirectBody::StreamRecord(_)))
}

/// How an attempt that failed with `e` ends: a locked, not-ready or
/// conflicting host is tried again later, and the reason is logged.
pub(crate) fn attempt_of(e: FriendError) -> Attempt {
    match e {
        FriendError::Locked | FriendError::NotReady(_) | FriendError::Conflict(_) => {
            tracing::debug!(reason = %e, "outbox row deferred");
            Attempt::Deferred
        }
        e => Attempt::Wedged(e.to_string()),
    }
}

/// Push a row's `outcome`; whether it stays behind
/// (`rekindle_outbox::holds_back`), so the rows after it are not attempted.
pub(crate) fn push_stops(outcomes: &mut Vec<Option<Attempt>>, outcome: Attempt) -> bool {
    let stop = matches!(outcome, Attempt::Deferred | Attempt::Wedged(_));
    outcomes.push(Some(outcome));
    stop
}

/// An entry no chain page holds alone can never be sent; the others of a
/// batch are not held back by it.
pub(crate) fn fits_page(entry: &[u8]) -> Result<(), Attempt> {
    rekindle_records::chain::page_runs(&[entry.to_vec()])
        .map(|_| ())
        .map_err(|e| Attempt::Wedged(format!("chain append: {e}")))
}

/// Settle the chained rows at `slots` of `outcomes` (in order) after a run
/// append (`dm::stream::append_noted`): the first `landed` are sent, the
/// next one ends as the error that stopped the rest, and the rows after it
/// were not attempted.
pub(crate) fn settle_chained(
    outcomes: &mut Vec<Option<Attempt>>,
    slots: &[usize],
    landed: usize,
    appended: Result<(), FriendError>,
) {
    for i in slots.iter().take(landed) {
        outcomes[*i] = Some(Attempt::Sent);
    }
    if let (Err(e), Some(i)) = (appended, slots.get(landed)) {
        outcomes[*i] = Some(attempt_of(e));
        outcomes.truncate(*i + 1);
    }
}

/// The outcomes of the attempted prefix.
pub(crate) fn finish(outcomes: Vec<Option<Attempt>>) -> Vec<Attempt> {
    outcomes.into_iter().map_while(|o| o).collect()
}

/// Frame `row` afresh for this attempt.
async fn frame_row(
    host: &dyn FriendHost,
    to: &str,
    row: &RowAttempt<'_>,
) -> Result<Framed, Attempt> {
    let body = decode(row.payload).map_err(|e| Attempt::Wedged(format!("payload: {e}")))?;
    let seq = u64::try_from(row.stream_seq).unwrap_or(0);
    let sent_at_ms = row
        .sent_at
        .and_then(|t| u64::try_from(t).ok())
        .unwrap_or_else(|| host.clock().now_ms());
    let frame = send::stream_frame(host, to, &body, seq, sent_at_ms)
        .await
        .map_err(attempt_of)?;
    let datagram = super::frame::wire(&frame).map_err(|e| Attempt::Wedged(e.to_string()))?;
    if matches!(body, DirectBody::StreamRecord(_)) {
        return Ok(Framed::Record(datagram, frame.type_id));
    }
    fits_page(&datagram)?;
    Ok(Framed::Entry(datagram, frame.type_id))
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
