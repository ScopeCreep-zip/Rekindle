//! The `friend_handshake` stream on the outbox (plan E2.5, D22), as the
//! transport sees it: a friend request, accept or reject, a removal and
//! its ack, a session reset, a presence key or a rotation notice to one
//! peer is framed once, committed as a row of `friend_handshake:<peer>`,
//! and sent by the outbox until it settles. These are control frames no
//! `DmAck` answers: a row settles `sent` when its frame leaves, a request
//! and a first contact (the PreKeyInit carrying our `FriendAccept` or
//! `SessionResetAccept`) only on its `app_call` receipt (`delivered`: an
//! app message has no delivery guarantee, and a request or an accept lost
//! on the way would leave the row settled and the peer never asked, or
//! pending for ever; live A/B finding 27), and the `Unfriended` row waits
//! for the peer's `UnfriendedAck` (or fails at its deadline), which ends
//! the removal. An invite's redemption is a request row too, whose send
//! is the write into the invite's inbox ([`enqueue_inbox`]).

use rekindle_codec::capnp_codec::direct::{DirectBody, DmFrame, DmTypeId};
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::outbox::Attempt;
use super::send::{self, Framed};
use crate::operations::friend::{FriendError, FriendHost};

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

/// A fresh client id for a handshake row carrying `kind`.
fn client_id(host: &dyn FriendHost, kind: DmTypeId) -> String {
    let mut id = [0u8; 16];
    host.rng().fill(&mut id);
    format!("{kind:?}:{}", hex::encode(id))
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
    let (_, framed) = send::framed(host, to, body).await?;
    encode(&framed)
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
    let payload = payload(host, to, body).await?;
    commit(host, to, client_id(host, body.type_id()), payload).await
}

/// Commit a frame built elsewhere (a PreKeyInit opened from a bundle).
///
/// # Errors
/// The frame is over the cap, or the row could not be stored.
pub async fn enqueue_frame(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
) -> Result<String, FriendError> {
    let payload = encode(&Framed::Wire {
        datagram: super::frame::wire(frame)?,
    })?;
    commit(host, to, client_id(host, frame.type_id), payload).await
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

/// The reason an inbox write fails with when the inbox already holds
/// another requester's request.
pub const INBOX_TAKEN: &str = "the invite was already used";

/// One write of an invite's redemption: stored (or already ours) settles
/// it `sent`; a miss is attempted again; an inbox holding another value
/// fails it visibly ([`INBOX_TAKEN`]), as does a write the schema or
/// writer refuses ([`refuses`]); any other error is attempted again.
async fn attempt_inbox(
    host: &dyn FriendHost,
    record_key: &str,
    writer: &str,
    frame: &[u8],
) -> Attempt {
    use rekindle_records::port::SetOutcome;
    let Ok(Ok(records)) = host.node().map(|node| node.require_records()) else {
        return Attempt::Deferred;
    };
    let subkey = rekindle_types::dht_layout::request_inbox::REQUEST;
    match records
        .write_once(record_key, subkey, frame.to_vec(), Some(writer))
        .await
    {
        Ok(SetOutcome::Landed | SetOutcome::Unchanged) => Attempt::Sent,
        Ok(SetOutcome::Superseded(value)) if value.data == frame => Attempt::Sent,
        Ok(SetOutcome::Superseded(_)) => Attempt::Wedged(INBOX_TAKEN.to_owned()),
        Ok(SetOutcome::BelowConsensus | SetOutcome::Offline) => Attempt::Deferred,
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
fn refuses(e: &rekindle_records::port::PortError) -> bool {
    use rekindle_records::port::PortError;
    matches!(
        e,
        PortError::Dht(_)
            | PortError::NotWritable(_)
            | PortError::SubkeyTooLarge { .. }
            | PortError::MessageTooLarge { .. }
            | PortError::KeyText(_)
            | PortError::Codec(_)
            | PortError::Crypto(_)
    )
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
    tracing::debug!(to = %to, %client_id, "handshake frame committed to the outbox");
    Ok(client_id)
}

/// Whether a handshake row of `kind` goes by `app_call` and settles only
/// on its receipt: a request, and a first contact. Every PreKeyInit on
/// this stream is a first contact ([`enqueue_frame`]'s callers), and the
/// receiver answers a re-sent one it already opened with the same receipt
/// (`direct::receive_call`: a duplicate init is `Dropped::Duplicate`).
#[must_use]
pub fn receipted(kind: DmTypeId) -> bool {
    matches!(
        kind,
        DmTypeId::FriendRequest | DmTypeId::FriendAccept | DmTypeId::SessionResetAccept
    )
}

/// One attempt of a handshake row to `to`: a request or a first contact
/// ([`receipted`]) goes by `app_call` only, over the cached route or one
/// refreshed from their profile, and is attempted again until its empty
/// return (the receipt) comes back or its deadline passes; any other frame
/// is sent once over the route.
pub async fn attempt(host: &dyn FriendHost, to: &str, payload: &[u8]) -> Attempt {
    let framed: Framed = match serde_json::from_slice(payload) {
        Ok(framed) => framed,
        Err(e) => return Attempt::Wedged(format!("payload: {e}")),
    };
    if let Framed::Inbox {
        record_key,
        writer,
        frame,
    } = &framed
    {
        return attempt_inbox(host, record_key, writer, frame).await;
    }
    let (datagram, kind) = match send::datagram_of(host, to, &framed) {
        Ok(sent) => sent,
        Err(FriendError::Locked | FriendError::NotReady(_)) => return Attempt::Deferred,
        Err(e) => return Attempt::Wedged(e.to_string()),
    };
    if receipted(kind) {
        match send::call_datagram_answer(host, to, datagram).await {
            Ok(send::CallAnswer::Receipt | send::CallAnswer::Reply(_)) => {}
            // A first contact the peer refused is answered by its
            // `PreKeyRejected`, which sends the one retry (plan E2.4) on
            // this stream: this row ends and the rows behind it go on.
            Ok(send::CallAnswer::Refused) if kind != DmTypeId::FriendRequest => {
                return Attempt::Superseded(format!("{kind:?} refused by {to}"));
            }
            Ok(send::CallAnswer::Refused) => {
                tracing::debug!(to = %to, ?kind, "friend request refused by app_call — attempted again");
                return Attempt::Deferred;
            }
            Err(e) => {
                tracing::debug!(to = %to, ?kind, error = %e,
                    "handshake frame not receipted by app_call — attempted again");
                return Attempt::Deferred;
            }
        }
        if kind == DmTypeId::FriendRequest {
            host.publish(SubscriptionEvent::Friend(
                FriendEvent::RequestAcknowledged {
                    peer_key: to.to_string(),
                },
            ));
        }
        return Attempt::Receipted;
    }
    match send::route_send_datagram(host, to, &datagram, kind, true).await {
        Ok(true) if kind == DmTypeId::Unfriended => Attempt::Awaiting,
        Ok(true) => Attempt::Sent,
        Ok(false) | Err(FriendError::NotReady(_) | FriendError::Locked) => Attempt::Deferred,
        Err(e) => Attempt::Wedged(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inbox_write_fails_only_when_refused() {
        use rekindle_records::port::PortError;
        assert!(refuses(&PortError::NotWritable("rk".into())));
        assert!(refuses(&PortError::Dht(
            "InvalidArgument: wrong writer".into()
        )));
        assert!(!refuses(&PortError::Unreachable("TryAgain".into())));
        assert!(!refuses(&PortError::Closed));
        assert!(!refuses(&PortError::Node("not attached".into())));
        assert!(!refuses(&PortError::LeaseNotHeld(3)));
    }

    #[test]
    fn requests_and_first_contacts_wait_for_their_receipt() {
        assert!(receipted(DmTypeId::FriendRequest));
        assert!(receipted(DmTypeId::FriendAccept));
        assert!(receipted(DmTypeId::SessionResetAccept));
        assert!(!receipted(DmTypeId::FriendReject));
        assert!(!receipted(DmTypeId::Unfriended));
        assert!(!receipted(DmTypeId::FriendsKey));
    }

    #[test]
    fn the_unfriended_row_names_its_peer() {
        let id = unfriended_client_id("ab12");
        assert_eq!(unfriended_peer(&id), Some("ab12"));
        assert_eq!(unfriended_peer("FriendRequest:00"), None);
    }
}
