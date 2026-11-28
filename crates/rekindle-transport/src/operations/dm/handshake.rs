//! The `friend_handshake` stream on the outbox (plan E2.5, D22), as the
//! transport sees it: a friend request, accept or reject, a removal and
//! its ack, a session reset, a presence key or a rotation notice to one
//! peer is framed once, committed as a row of `friend_handshake:<peer>`,
//! and sent by the outbox until it settles. Rows needing confirmation
//! settle `delivered` on an `app_call` receipt ([`receipted`]) or wait for
//! a separate answer ([`awaits_answer`]). A one-way route send can settle
//! `sent` as soon as the frame leaves, even if the route loses it (live
//! A/B finding 27). An invite redemption is a request row whose send is
//! the write into the invite's inbox ([`enqueue_inbox`]).

use rekindle_codec::capnp_codec::direct::{DirectBody, DmFrame, DmTypeId};
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::outbox::Attempt;
use super::send::{self, Framed};
use crate::operations::friend::{FriendError, FriendHost};
use crate::operations::prekeys::Rejection;

pub use super::handshake_order::{
    restore_first_contacts, settle_first_contact, takes_over, waits_for,
};

/// The client id of our `Unfriended` row to `peer`: the one row their
/// `UnfriendedAck` settles.
#[must_use]
pub fn unfriended_client_id(peer: &str) -> String {
    format!("unfriended:{peer}")
}

/// Whether `client_id` is an `Unfriended` row's ([`unfriended_client_id`]);
/// the peer it names.
#[must_use]
pub fn unfriended_peer(client_id: &str) -> Option<&str> {
    client_id.strip_prefix("unfriended:")
}

/// The prefix every handshake row carrying `kind` has on its client id.
#[must_use]
pub fn client_prefix(kind: DmTypeId) -> String {
    format!("{kind:?}:")
}

/// A fresh client id for a handshake row carrying `kind`.
fn client_id(host: &dyn FriendHost, kind: DmTypeId) -> String {
    let mut id = [0u8; 16];
    host.rng().fill(&mut id);
    format!("{}{}", client_prefix(kind), hex::encode(id))
}

fn encode(framed: &Framed) -> Result<Vec<u8>, FriendError> {
    serde_json::to_vec(framed)
        .map_err(|e| FriendError::Failed(format!("serialize the handshake frame: {e}")))
}

/// `body` to `to` framed as a handshake row stores it.
///
/// # Errors
/// The frame could not be built (the verification gate, no session for a
/// ratchet kind, over the cap).
pub async fn payload(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<Vec<u8>, FriendError> {
    Ok(payload_of(host, to, body).await?.0)
}

/// [`payload`] and the kind its row is named by: `PreKeyInit` for a group
/// member's first contact ([`super::group_first_contact::framed`]).
async fn payload_of(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<(Vec<u8>, DmTypeId), FriendError> {
    let (frame, framed) = send::framed(host, to, body).await?;
    let (framed, kind) = super::group_first_contact::framed(host, to, body, &frame, framed).await?;
    Ok((encode(&framed)?, kind))
}

/// Frame `body` and commit it to our handshake stream to `to`; the row's
/// client id. It returns once the row is stored: the outbox sends it.
///
/// # Errors
/// The frame could not be built, or the row could not be stored.
pub async fn enqueue(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<String, FriendError> {
    let (payload, kind) = payload_of(host, to, body).await?;
    commit(host, to, client_id(host, kind), payload).await
}

/// [`enqueue`] for a frame worth nothing after `deadline_ms` (ms since the
/// epoch): a call's `CallEnd` (plan CE1, decision 22) is retried while its
/// peer is unreachable, brought forward when they are reachable again
/// ([`super::route::reachable_again`]), and fails at the deadline. It
/// returns once the row is stored.
///
/// # Errors
/// The frame could not be built, or the row could not be stored.
pub async fn enqueue_until(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
    deadline_ms: i64,
) -> Result<String, FriendError> {
    let payload = payload(host, to, body).await?;
    let client_id = client_id(host, body.type_id());
    let outbox = host.outbox()?;
    outbox
        .enqueue_handshake_until(to, client_id.clone(), payload, deadline_ms)
        .await?;
    // A call's end supersedes its own invite still unsettled (plan HOL1,
    // decision 47): nothing is left to ring for.
    if let DirectBody::CallEnd(closed) = body {
        outbox
            .discard(&super::call_invite::client_id(&closed.call_id))
            .await?;
    }
    tracing::debug!(to = %to, %client_id, deadline_ms, "deadlined handshake frame committed");
    Ok(client_id)
}

/// [`enqueue`], except that a frame whose session cannot be opened yet
/// (`NotReady`: the member's prekey record not readable, plan E2.6 finding
/// GA1) is committed to wait as its body ([`Framed::Pending`]), framed at
/// an attempt once the session opens: a group frame is committed at once,
/// never refused for "not yet". Any other failure is returned.
///
/// # Errors
/// The frame could not be built for a reason other than `NotReady`, or
/// the row could not be stored.
pub async fn enqueue_when_ready(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<String, FriendError> {
    let mut kind = body.type_id();
    let payload = match payload_of(host, to, body).await {
        Ok((payload, row)) => {
            kind = row;
            payload
        }
        Err(e @ FriendError::NotReady(_)) => {
            tracing::info!(?kind, "group frame waits for the member's session to open");
            tracing::debug!(to = %to, ?kind, reason = %e, "frame committed to wait");
            encode(&Framed::Pending {
                type_id: kind.as_u16(),
                body: body.encode()?,
            })?
        }
        Err(e) => return Err(e),
    };
    commit(host, to, client_id(host, kind), payload).await
}

/// Commit a frame built elsewhere (a PreKeyInit opened from a bundle); a
/// `FriendAccept` init to a requester that named a reply inbox is written
/// there too (plan RV2, `first_contact_durable`).
///
/// # Errors
/// The frame is over the cap, or the row could not be stored.
pub async fn enqueue_frame(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
) -> Result<String, FriendError> {
    let datagram = super::frame::wire(frame)?;
    let framed = super::first_contact_durable::framed(host, to, frame, datagram).await?;
    commit(host, to, client_id(host, frame.type_id), encode(&framed)?).await
}

/// Commit an invite's redemption to our handshake stream to `to`, the
/// inviter: `frame` (our sealed request) written into the inbox
/// `record_key` as its one-time `writer` by the outbox. It returns once
/// the row is stored.
///
/// # Errors
/// The row could not be stored.
pub async fn enqueue_inbox(
    host: &dyn FriendHost,
    to: &str,
    record_key: &str,
    writer: &str,
    frame: Vec<u8>,
) -> Result<String, FriendError> {
    let payload = encode(&Framed::Inbox {
        record_key: record_key.to_owned(),
        writer: writer.to_owned(),
        frame,
    })?;
    commit(host, to, client_id(host, DmTypeId::FriendRequest), payload).await
}

/// Commit a first contact of `kind` that waits for `to`'s prekey record
/// (live finding L-FR3): attempted with the stream's backoff, and brought
/// forward when their profile names a new device list
/// (`route::reachable_again`). It returns once the row is stored.
///
/// # Errors
/// The row could not be stored.
pub async fn enqueue_first_contact(
    host: &dyn FriendHost,
    to: &str,
    kind: DmTypeId,
) -> Result<String, FriendError> {
    let payload = encode(&Framed::FirstContact {
        first_type_id: kind.as_u16(),
    })?;
    commit(host, to, client_id(host, kind), payload).await
}

/// The reason an inbox write fails with when the inbox already holds
/// another requester's request.
pub const INBOX_TAKEN: &str = "the invite was already used";

/// One write of an invite's redemption: stored (or already ours, which the
/// pool reports as `Landed` or `Unchanged` even over a rival) settles it
/// `sent`; a miss is attempted again; an inbox holding another value
/// fails it visibly ([`INBOX_TAKEN`]), as does a write the schema or
/// writer refuses ([`refuses`]); any other error is attempted again.
pub(super) async fn attempt_inbox(
    host: &dyn FriendHost,
    record_key: &str,
    writer: &str,
    frame: &[u8],
) -> Attempt {
    use rekindle_records::port::SetOutcome;
    let Ok(Ok(records)) = host.node().map(|node| node.require_records()) else {
        tracing::debug!("inbox write deferred: the node or its record port is not up");
        return Attempt::Deferred;
    };
    let subkey = rekindle_types::dht_layout::request_inbox::REQUEST;
    match records
        .write_once(record_key, subkey, frame.to_vec(), Some(writer))
        .await
    {
        Ok(SetOutcome::Landed | SetOutcome::Unchanged) => Attempt::Sent,
        Ok(SetOutcome::Superseded(_)) => Attempt::Wedged(INBOX_TAKEN.to_owned()),
        Ok(outcome @ SetOutcome::Offline) => {
            tracing::debug!(?outcome, "inbox write deferred");
            Attempt::Deferred
        }
        Err(e) if refuses(&e) => Attempt::Wedged(format!("inbox write refused: {e}")),
        Err(e) => {
            tracing::debug!(error = %e, "inbox write missed — attempted again");
            Attempt::Deferred
        }
    }
}

/// Whether an inbox write's error refuses it for good: the schema or the
/// writer (Veilid's "wrong writer" is an `InvalidArgument`
/// (`storage_manager/schema.rs` `check_subkey_value_data`), which the pool
/// reports as `Dht` because it is not transient (`dht/pool/mod.rs`
/// `transient`; the netsim's `SimDht` refuses the same way, plan E2.B S3),
/// or a value that can never be written. The network's misses, a closed pool
/// and the node not being ready are attempted again.
pub(crate) fn refuses(e: &rekindle_records::port::PortError) -> bool {
    use rekindle_records::port::PortError;
    matches!(
        e,
        PortError::Dht(_)
            | PortError::NotWritable(_)
            | PortError::SubkeyTooLarge { .. }
            | PortError::MessageTooLarge { .. }
            | PortError::KeyText(_)
            | PortError::Codec(_)
            | PortError::TooLargeToEncode(_)
            | PortError::Crypto(_)
    )
}

/// A handshake row framed now and committed later ([`commit_framed`]):
/// what a frame needs when its session is about to go (our
/// `UnfriendedAck`, whose removal deletes the session it is framed under).
pub struct FramedRow {
    kind: DmTypeId,
    payload: Vec<u8>,
}

/// Frame `body` to `to` under the session as it is now.
///
/// # Errors
/// The frame could not be built.
pub async fn frame_now(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<FramedRow, FriendError> {
    Ok(FramedRow {
        kind: body.type_id(),
        payload: payload(host, to, body).await?,
    })
}

/// Commit a row [`frame_now`] framed; the row's client id.
///
/// # Errors
/// The row could not be stored.
pub async fn commit_framed(
    host: &dyn FriendHost,
    to: &str,
    row: FramedRow,
) -> Result<String, FriendError> {
    commit(host, to, client_id(host, row.kind), row.payload).await
}

async fn commit(
    host: &dyn FriendHost,
    to: &str,
    client_id: String,
    payload: Vec<u8>,
) -> Result<String, FriendError> {
    host.outbox()?
        .enqueue_handshake(to, client_id.clone(), payload)
        .await?;
    super::handshake_order::committed(host, to, &client_id);
    tracing::debug!(to = %to, %client_id, "handshake frame committed to the outbox");
    Ok(client_id)
}

/// Whether a handshake row of `kind` goes by `app_call` and settles only
/// on its receipt. The receiver answers a sealed reset once its handler
/// succeeds. For ratchet frames, the receiver answers once the ratchet
/// step and the unprocessed row are committed (`direct::receive_call`).
/// A duplicate gets the same empty receipt. The remover keeps the session
/// while its removal waits (`removing`), so it opens our ack, settles its
/// `Unfriended` row (`acknowledge_unfriended`, a no-op for a duplicate
/// whose row already settled) and answers the receipt (finding 62). Every PreKeyInit on
/// this stream is a first contact ([`enqueue_frame`]'s callers), and the
/// receiver answers a re-sent one it already opened with the same receipt
/// (`direct::receive_call`: a duplicate init is `Dropped::Duplicate`).
/// A group member's key (`GroupDmAccept`) and the creator's
/// `GroupDmInvite` and `GroupDmRosterGrow` are receipted too (plan E2.6).
/// A lost frame can leave a member without a key, chain or signed roster. A
/// `FriendsKey` and these group kinds come here only to a peer not yet an
/// accepted friend: to an accepted friend they are entries of our DM
/// chain, settled by the friend's `DmAck` (plan RV2c, D22,
/// [`super::friend_control`]), so a stalled call no longer holds them.
/// A call's `CallEnd` is receipted (plan CE4, lead decision 22): sent as
/// an `app_message` it settled on leaving and was lost unseen by a frozen
/// peer (Veilid 0.5.8 `rpc_app_message.rs:24`: a statement no one
/// answers), so it waits for the receipt, re-sent until its deadline; the
/// receiver answers once open, a duplicate with the same receipt
/// (`Dropped::Duplicate`), and ignores a call id it does not hold.
#[must_use]
pub fn receipted(kind: DmTypeId) -> bool {
    matches!(
        kind,
        DmTypeId::FriendRequest
            | DmTypeId::FriendAccept
            | DmTypeId::SessionResetRequest
            | DmTypeId::SessionResetAccept
            | DmTypeId::SessionResetDecline
            | DmTypeId::FriendsKey
            | DmTypeId::UnfriendedAck
            | DmTypeId::GroupDmInvite
            | DmTypeId::GroupDmAccept
            | DmTypeId::GroupDmRosterGrow
            | DmTypeId::CallEnd
    )
}

/// What a receipted row's `app_call` answer means.
#[derive(Debug, PartialEq, Eq)]
enum Answered {
    /// The empty return: the receipt.
    Receipt,
    /// Their `PreKeyRejected`, opened and applied (B5, finding 46): it sent
    /// the one resend (plan E2.4), so this row ends.
    Rejected,
    /// A bare NAK, or a reply that did not open or was not applied (a
    /// `PreKeyRejected` naming no unanswered init of ours): a refusal,
    /// shown on the friend's row as failed with Retry (finding 46 refined,
    /// `first_contact::refused`), never a receipt or a silent split.
    Refused,
    /// A friend request's bare NAK (LV23 review M1): never a block (a block
    /// is the silent receipt), so attempted again as [`Self::Again`] is, the
    /// row showing a generic "couldn't deliver, will retry"
    /// (`first_contact::request_undelivered`), never "did not accept".
    Undelivered,
    /// No answer, their "retry later" (RV5 M2), a friend request's reply
    /// (no answer a request defines), or our own failure applying a reply:
    /// attempted again until the row's deadline.
    Again,
}

/// [`Answered`] for `kind`'s `answer`, where `rejection` is what its reply
/// came to as a `PreKeyRejected` (`direct::on_first_contact_reply`).
fn answered(
    kind: DmTypeId,
    answer: &Result<send::CallAnswer, FriendError>,
    rejection: Rejection,
) -> Answered {
    match answer {
        Ok(send::CallAnswer::Receipt) => Answered::Receipt,
        Ok(send::CallAnswer::Reply(_)) if kind == DmTypeId::FriendRequest => Answered::Again,
        Ok(send::CallAnswer::Refused) if kind == DmTypeId::FriendRequest => Answered::Undelivered,
        Ok(send::CallAnswer::Reply(_)) if rejection == Rejection::Applied => Answered::Rejected,
        // Our own failure judging or applying it is no refusal of theirs
        // (RV5 R5): attempted again.
        Ok(send::CallAnswer::Reply(_)) if rejection == Rejection::Failed => Answered::Again,
        Ok(send::CallAnswer::Reply(_) | send::CallAnswer::Refused) => Answered::Refused,
        // Their "retry later" is their own fault, said at once (review RV5
        // M2): attempted again as a timeout is.
        Ok(send::CallAnswer::RetryLater) | Err(_) => Answered::Again,
    }
}

/// What a failed attempt's answer was, without the peer or the error text
/// (which may name a route): for the INFO line.
fn answer_outcome(answer: &Result<send::CallAnswer, FriendError>) -> &'static str {
    match answer {
        Ok(send::CallAnswer::RetryLater) => "retry later",
        Ok(_) => "reply",
        Err(FriendError::NotReady(_)) => "no usable route",
        Err(_) => "call failed",
    }
}

/// One attempt of a handshake row to `to`: a request or a first contact ([`receipted`]) goes by
/// `app_call` (also by its write into the requester's reply inbox or the code's slot: RV2, RV2e),
/// over the cached route or one refreshed from their profile, again until its receipt comes back,
/// a judged refusal ends it ([`answered`]) or its deadline passes; any other frame goes once.
pub async fn attempt(host: &dyn FriendHost, to: &str, payload: &[u8]) -> Attempt {
    let framed: Framed = match serde_json::from_slice(payload) {
        Ok(framed) => framed,
        Err(e) => return Attempt::Wedged(format!("payload: {e}")),
    };
    if framed.is_request() {
        if let Some(stopped) = crate::operations::friend::inbox::reply_slot(host, to).await {
            return stopped;
        }
    }
    match &framed {
        Framed::Inbox {
            record_key,
            writer,
            frame,
        } => {
            // An inbox row is a friend request (`friend::inbox::redeem`):
            // the chain it names is published first (decision 67 item 2).
            let kind = DmTypeId::FriendRequest;
            if let Some(stopped) =
                super::first_contact_durable::chain_published(host, to, kind).await
            {
                return stopped;
            }
            return attempt_inbox(host, record_key, writer, frame).await;
        }
        Framed::FirstContactDurable {
            datagram,
            record_key,
            writer,
        } => {
            return super::first_contact_durable::attempt(host, to, datagram, record_key, writer)
                .await
        }
        Framed::GroupFirstContact { datagram, group_id } => {
            return super::group_first_contact::attempt(host, to, datagram, group_id).await;
        }
        Framed::FirstContact { first_type_id } => {
            return super::first_contact_durable::attempt_waiting(host, to, *first_type_id).await;
        }
        Framed::Pending { type_id, body } => {
            return super::group_first_contact::attempt_pending(host, to, *type_id, body).await;
        }
        Framed::Wire { .. } | Framed::Sealed { .. } => {}
    }
    let (datagram, kind) = match send::datagram_of(host, to, &framed) {
        Ok(sent) => sent,
        Err(e @ (FriendError::Locked | FriendError::NotReady(_))) => {
            tracing::debug!(to = %to, reason = %e, "handshake row deferred");
            return Attempt::Deferred;
        }
        Err(e) => return Attempt::Wedged(e.to_string()),
    };
    if let Some(done) =
        super::first_contact_durable::code_request(host, to, &framed, &datagram).await
    {
        return done;
    }
    if let Some(stopped) = super::first_contact_durable::chain_published(host, to, kind).await {
        return stopped;
    }
    if receipted(kind) {
        return attempt_call(host, to, datagram, kind).await;
    }
    match send::route_send_datagram(host, to, &datagram, kind).await {
        Ok(true) if awaits_answer(kind) => Attempt::Awaiting,
        Ok(true) => Attempt::Sent,
        Ok(false) => {
            tracing::debug!(to = %to, ?kind, "handshake row deferred: no usable route");
            Attempt::Deferred
        }
        Err(e @ (FriendError::NotReady(_) | FriendError::Locked)) => {
            tracing::debug!(to = %to, ?kind, reason = %e, "handshake row deferred");
            Attempt::Deferred
        }
        Err(e) => Attempt::Wedged(e.to_string()),
    }
}

/// One `app_call` of a receipted row's `datagram` of `kind` to `to`, and
/// what its answer comes to ([`answered`]).
pub(super) async fn attempt_call(
    host: &dyn FriendHost,
    to: &str,
    datagram: Vec<u8>,
    kind: DmTypeId,
) -> Attempt {
    let answer = send::call_datagram_answer(host, to, datagram).await;
    let applied = match &answer {
        Ok(send::CallAnswer::Reply(reply)) if kind != DmTypeId::FriendRequest => {
            crate::operations::direct::on_first_contact_reply(host, to, reply).await
        }
        _ => Rejection::NotOurs,
    };
    match answered(kind, &answer, applied) {
        Answered::Receipt => {
            if kind == DmTypeId::FriendRequest {
                crate::operations::prekeys::request_delivered(host, to);
                host.publish(SubscriptionEvent::Friend(
                    FriendEvent::RequestAcknowledged {
                        peer_key: to.to_string(),
                    },
                ));
            }
            Attempt::Receipted
        }
        Answered::Rejected => Attempt::Superseded(format!("{kind:?} prekeys refused by {to}")),
        Answered::Refused => {
            if matches!(kind, DmTypeId::FriendAccept | DmTypeId::SessionResetAccept) {
                crate::operations::prekeys::refused(host, to, kind);
            }
            Attempt::Superseded(format!("{kind:?} refused by {to}"))
        }
        Answered::Undelivered => {
            tracing::debug!(to = %to, "friend request answered with a NAK — attempted again");
            crate::operations::prekeys::request_undelivered(host, to);
            Attempt::Deferred
        }
        Answered::Again => {
            if kind == DmTypeId::FriendRequest {
                // Identifier-free at INFO (live finding LF5): a request
                // that does not land is seen retrying, never silent.
                tracing::info!(
                    outcome = answer_outcome(&answer),
                    "friend request attempt failed — attempted again"
                );
            }
            if let Err(e) = &answer {
                tracing::debug!(to = %to, ?kind, error = %e,
                    "handshake frame not receipted by app_call — attempted again");
            } else {
                tracing::debug!(to = %to, ?kind,
                    "friend request answered with a reply — attempted again");
            }
            Attempt::Deferred
        }
    }
}

/// Whether a frame sent over the route stays unsettled until the peer
/// answers it, re-sent meanwhile with the stream's backoff and when the
/// peer is reachable again, until its deadline: an `Unfriended` (its
/// `UnfriendedAck`), and a `CallInvite` (plan CE3, lead decision 30: its
/// `CallRinging`, accept or decline; `call_invite::settle`), since an
/// `app_message` over a stale route is lost unseen.
#[must_use]
pub fn awaits_answer(kind: DmTypeId) -> bool {
    matches!(kind, DmTypeId::Unfriended | DmTypeId::CallInvite)
}

#[cfg(test)]
#[path = "handshake_tests.rs"]
mod tests;
