//! The `friend_handshake` stream on the outbox (plan E2.5, D22), as the
//! transport sees it: a friend request, accept or reject, a removal and
//! its ack, a session reset, a presence key or a rotation notice to one
//! peer is framed once, committed as a row of `friend_handshake:<peer>`,
//! and sent by the outbox until it settles. These are control frames no
//! `DmAck` answers: a row settles `sent` when its frame leaves, a request
//! only on its `app_call` receipt (`delivered`: an app message has no
//! delivery guarantee, and a request lost on the way would leave the row
//! settled and the peer never asked), and the `Unfriended` row waits
//! for the peer's `UnfriendedAck` (or fails at its deadline), which ends
//! the removal.

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

/// One attempt of a handshake row to `to`: a request goes by `app_call`
/// only, over the cached route or one refreshed from their profile, and is
/// attempted again until its empty return (the receipt) comes back; any
/// other frame is sent once over the route.
pub async fn attempt(host: &dyn FriendHost, to: &str, payload: &[u8]) -> Attempt {
    let framed: Framed = match serde_json::from_slice(payload) {
        Ok(framed) => framed,
        Err(e) => return Attempt::Wedged(format!("payload: {e}")),
    };
    let (datagram, kind) = match send::datagram_of(host, to, &framed) {
        Ok(sent) => sent,
        Err(FriendError::Locked | FriendError::NotReady(_)) => return Attempt::Deferred,
        Err(e) => return Attempt::Wedged(e.to_string()),
    };
    if kind == DmTypeId::FriendRequest {
        match send::call_datagram(host, to, datagram.clone()).await {
            Ok(_) => {
                host.publish(SubscriptionEvent::Friend(
                    FriendEvent::RequestAcknowledged {
                        peer_key: to.to_string(),
                    },
                ));
                return Attempt::Receipted;
            }
            Err(e) => {
                tracing::debug!(to = %to, error = %e,
                    "friend request not receipted by app_call — attempted again");
                return Attempt::Deferred;
            }
        }
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
    fn the_unfriended_row_names_its_peer() {
        let id = unfriended_client_id("ab12");
        assert_eq!(unfriended_peer(&id), Some("ab12"));
        assert_eq!(unfriended_peer("FriendRequest:00"), None);
    }
}
