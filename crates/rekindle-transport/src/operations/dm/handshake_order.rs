//! What a peer's `friend_handshake` row waits for (plan HOL1, lead decision
//! 47), and the implicit acknowledgement of our first contact: the peer's
//! first frame opened under the session settles our unsettled PreKeyInit
//! row (libsignal `tahini.rs` `on_receive_session_state`). Only a peer with
//! such a row unsettled costs an outbox query on a received frame
//! ([`FriendStores::unsettled_first_contacts`], set when the row is
//! committed, rebuilt from the outbox at unlock, cleared at settle).

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::DmTypeId;
use rekindle_types::message::SendState;

use super::handshake::{client_prefix, unfriended_peer};
use super::outbox::DmOutbox;
use crate::operations::friend::{state, FriendError, FriendHost, FriendStores};

/// The stream prefix of a peer's handshake rows.
const HANDSHAKE_STREAM: &str = "friend_handshake:";

/// The kinds a first contact's row is named by: the `PreKeyInit` carrying
/// our `FriendAccept` or `SessionResetAccept` (`handshake::enqueue_frame`
/// names a row by the frame's own kind, and every `PreKeyInit` on this
/// stream is a first contact: `dm/outgoing.rs` `send_friend_accept`,
/// `prekeys/first_contact.rs`), and the `FriendAccept` or
/// `SessionResetAccept` row still waiting for the peer's prekey record
/// (`handshake::enqueue_first_contact`). HOL1g: without the `PreKeyInit`
/// prefix a real first contact was never noted and nothing waited for it.
const FIRST_CONTACT: [DmTypeId; 3] = [
    DmTypeId::PreKeyInit,
    DmTypeId::FriendAccept,
    DmTypeId::SessionResetAccept,
];

/// The kinds whose row waits for no other row of the stream (plan HOL1,
/// lead decision 47): a first contact and a request themselves, a session
/// reset's frames, a removal and its ack, a group's and a creator round's
/// frames (roster admission, not the friendship, admits them; a ratchet
/// frame among them waits for the session as `Framed::Pending`), and a
/// call's end. Every other frame needs the peer to have applied our
/// friendship and to hold our session (the receiver drops it otherwise:
/// `friend/presence_key.rs`, `direct/frame.rs` NoSession), so it waits for
/// our unsettled first contact ([`waits_for`]).
const INDEPENDENT: [DmTypeId; 22] = [
    DmTypeId::FriendRequest,
    DmTypeId::PreKeyInit,
    DmTypeId::FriendAccept,
    DmTypeId::SessionResetRequest,
    DmTypeId::SessionResetAccept,
    DmTypeId::SessionResetDecline,
    DmTypeId::PreKeyRejected,
    DmTypeId::Unfriended,
    DmTypeId::UnfriendedAck,
    DmTypeId::GroupDmInvite,
    DmTypeId::GroupDmAccept,
    DmTypeId::GroupDmDecline,
    DmTypeId::GroupDmLeave,
    DmTypeId::GroupContent,
    DmTypeId::CreatorInvite,
    DmTypeId::CreatorAccept,
    DmTypeId::CreatorDecline,
    DmTypeId::CreatorWithdrawn,
    DmTypeId::CreatorJoin,
    DmTypeId::CallEnd,
    DmTypeId::GroupCallOffer,
    DmTypeId::GroupCallParticipantLeft,
];

/// The client-id prefixes of the row a handshake row `client_id` waits for
/// (plan HOL1, lead decision 47): a friend-gated frame waits for our
/// unsettled first contact to the peer (the `FriendAccept` or
/// `SessionResetAccept` PreKeyInit), resolved when the row is stored, and
/// nothing when there is none; an [`INDEPENDENT`] kind's and a removal's
/// (`unfriended:`) wait for nothing.
#[must_use]
pub fn waits_for(client_id: &str) -> Vec<String> {
    let independent = unfriended_peer(client_id).is_some()
        || INDEPENDENT
            .iter()
            .any(|kind| client_id.starts_with(&client_prefix(*kind)));
    if independent {
        return Vec::new();
    }
    FIRST_CONTACT.iter().map(|k| client_prefix(*k)).collect()
}

/// Whether `client_id` is a first contact's row.
fn is_first_contact(client_id: &str) -> bool {
    FIRST_CONTACT
        .iter()
        .any(|kind| client_id.starts_with(&client_prefix(*kind)))
}

/// Note `peer` as having an unsettled first contact when `client_id`, a
/// row of their handshake stream just stored or queued again, is one.
fn mark(stores: &FriendStores, peer: &str, client_id: &str) {
    if is_first_contact(client_id) {
        stores
            .unsettled_first_contacts
            .lock()
            .insert(peer.to_owned());
    }
}

/// A handshake row `client_id` to `to` was committed.
pub(super) fn committed(host: &dyn FriendHost, to: &str, client_id: &str) {
    mark(host.stores(), to, client_id);
}

/// The user's Retry queued row `client_id` again: a first contact's row
/// is unsettled once more, so its peer is noted again.
pub(crate) async fn retried(host: &dyn FriendHost, client_id: &str) {
    if !is_first_contact(client_id) {
        return;
    }
    let read = async {
        let owner = state::owner(host)?;
        let id = client_id.to_owned();
        state::db_call!(host, "read the retried row", move |conn| {
            rekindle_db::repo::outbox::get(conn, &owner, &id)
        })
        .await
    };
    match read.await {
        Ok(Some(row)) => {
            if let Some(peer) = row.stream.strip_prefix(HANDSHAKE_STREAM) {
                mark(host.stores(), peer, client_id);
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(%client_id, error = %e, "retried first contact not noted"),
    }
}

/// Rebuild [`FriendStores::unsettled_first_contacts`] from the outbox at
/// unlock: every peer with a first contact's row still unsettled.
///
/// # Errors
/// No identity is unlocked, or the storage failed.
pub async fn restore_first_contacts(host: &dyn FriendHost) -> Result<usize, FriendError> {
    let owner = state::owner(host)?;
    let rows = state::db_call!(host, "read the handshake rows", move |conn| {
        rekindle_db::repo::outbox::prefix_rows(conn, &owner, HANDSHAKE_STREAM)
    })
    .await?;
    let stores = host.stores();
    stores.unsettled_first_contacts.lock().clear();
    for row in rows.iter().filter(|r| r.state != SendState::Failed) {
        if let Some(peer) = row.stream.strip_prefix(HANDSHAKE_STREAM) {
            mark(stores, peer, &row.client_id);
        }
    }
    Ok(stores.unsettled_first_contacts.lock().len())
}

/// The peer opened a frame of theirs under our session: our first contact
/// to them (the PreKeyInit carrying our `FriendAccept` or
/// `SessionResetAccept`) arrived, whatever became of its receipt, so its
/// unsettled row settles `delivered` and the frames waiting for it go
/// (plan HOL1; libsignal stops wrapping messages as PreKey messages on the
/// first message received on the session, `tahini.rs` `on_receive_session_state`).
/// A peer with no first contact unsettled costs no outbox query.
pub async fn settle_first_contact(host: &dyn FriendHost, peer: &str) {
    settle_pending(host.stores(), || host.outbox(), peer).await;
}

/// [`settle_first_contact`] over `stores` and the outbox `outbox` gives.
async fn settle_pending(
    stores: &FriendStores,
    outbox: impl FnOnce() -> Result<Arc<dyn DmOutbox>, FriendError>,
    peer: &str,
) {
    if !stores.unsettled_first_contacts.lock().remove(peer) {
        return;
    }
    let Ok(outbox) = outbox() else {
        return;
    };
    for kind in FIRST_CONTACT {
        match outbox
            .receipt_handshakes_to(peer, &client_prefix(kind))
            .await
        {
            Ok(0) => {}
            Ok(settled) => tracing::debug!(peer = %peer, ?kind, settled,
                "first contact settled by the peer's frame under the session"),
            Err(e) => {
                // Not settled: the next frame tries again.
                stores
                    .unsettled_first_contacts
                    .lock()
                    .insert(peer.to_owned());
                tracing::warn!(peer = %peer, ?kind, error = %e, "first contact not settled");
            }
        }
    }
}

#[cfg(test)]
#[path = "handshake_order_tests.rs"]
mod tests;
