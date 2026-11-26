//! Control frames to an accepted friend ride the DM chain (plan RV2c, lead
//! decision 67, D22): a presence key (`FriendsKey`) and the pairwise group
//! kinds (`GroupDmInvite`, `GroupDmRosterGrow`, `GroupDmAccept`,
//! `GroupDmDecline`, `GroupDmLeave`) to a friend whose friendship is
//! `Accepted` are committed as control rows of `dm:<peer>`, not as
//! `friend_handshake` rows. Each is framed at its attempt under the
//! session, given its chain seq (`chain_seq::assign`), appended to our
//! chain (`stream::append`) and hinted by `app_message`; it settles
//! `delivered` when the friend's `DmAck` cursor passes it
//! (`deliver_chain_through`), and retires with the chain after that ack and
//! `DELIVERY_HORIZON` (`stream::retire`). A friend whose calls stall still
//! reads it, from the hint or from the chain on its watch, its unlock's
//! catch-up or the next hint's diff: the chain is authoritative (D23),
//! where a receipted `app_call` waited for an answer that did not come.
//!
//! The friend's order is kept by the stream: one stream per peer, its rows
//! framed, appended and hinted in `stream_seq` order with one append in
//! flight (VU2), and the receiver's cursor stops below an entry it cannot
//! hold yet (`direct::receive_entry`'s `Pending`, e.g. a frame from a
//! friend whose accept has not landed, `Dropped::NotYet`), so an entry is
//! never applied ahead of an earlier one of the same chain. Admission is
//! the one 1:1 path's (`direct::receive_entry` → `handlers::apply`): the
//! friend gate and the roster gate as for the call path.
//!
//! A frame to a peer not yet `Accepted` (a group member who is not our
//! friend, whose first contact opens a pairwise session) keeps the
//! `friend_handshake` path ([`handshake::enqueue`],
//! [`handshake::enqueue_when_ready`]). The kinds that end or must leave the
//! chain stay there too: `Unfriended`/`UnfriendedAck` end it, `CallEnd` is
//! deadlined (D8), and a session reset's frames are sealed or a PreKeyInit,
//! which the DM stream does not frame (`send::stream_frame`).

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};

use super::chat::{client_id_text, mint_client_id};
use super::handshake;
use crate::operations::friend::{state, FriendError, FriendHost};

/// Whether `kind` goes on the DM chain when its peer is an accepted friend.
#[must_use]
pub fn on_chain(kind: DmTypeId) -> bool {
    matches!(
        kind,
        DmTypeId::FriendsKey
            | DmTypeId::GroupDmInvite
            | DmTypeId::GroupDmRosterGrow
            | DmTypeId::GroupDmAccept
            | DmTypeId::GroupDmDecline
            | DmTypeId::GroupDmLeave
    )
}

/// Commit `body` to `to` as a control row of our DM stream when it is an
/// [`on_chain`] kind and `to` is an accepted friend; its client id, or
/// `None` when it goes by the handshake stream.
async fn to_chain(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<Option<String>, FriendError> {
    if !on_chain(body.type_id()) || !state::is_accepted(host, to) {
        return Ok(None);
    }
    let payload = super::outbox::payload(body)?;
    let client_id = client_id_text(&mint_client_id(host));
    host.outbox()?
        .enqueue_control(to, client_id.clone(), payload)
        .await?;
    tracing::debug!(to = %to, kind = ?body.type_id(), %client_id,
        "control frame committed to the dm chain");
    Ok(Some(client_id))
}

/// Commit `body` to `to`: on our DM chain to an accepted friend
/// ([`on_chain`]), else as a handshake row ([`handshake::enqueue`]). The
/// row's client id.
///
/// # Errors
/// The frame could not be built, or the row could not be stored.
pub async fn enqueue(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<String, FriendError> {
    match to_chain(host, to, body).await? {
        Some(client_id) => Ok(client_id),
        None => handshake::enqueue(host, to, body).await,
    }
}

/// [`enqueue`] for a group frame: to a member who is not an accepted
/// friend it is [`handshake::enqueue_when_ready`] (committed to wait for a
/// session that cannot open yet, finding GA1).
///
/// # Errors
/// The frame could not be built, or the row could not be stored.
pub async fn enqueue_when_ready(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<String, FriendError> {
    match to_chain(host, to, body).await? {
        Some(client_id) => Ok(client_id),
        None => handshake::enqueue_when_ready(host, to, body).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_friend_control_kinds_go_on_the_chain() {
        for kind in [
            DmTypeId::FriendsKey,
            DmTypeId::GroupDmInvite,
            DmTypeId::GroupDmRosterGrow,
            DmTypeId::GroupDmAccept,
            DmTypeId::GroupDmDecline,
            DmTypeId::GroupDmLeave,
        ] {
            assert!(on_chain(kind), "{kind:?} rides the dm chain");
            assert_eq!(
                kind.carrier(),
                rekindle_codec::capnp_codec::direct::Carrier::Ratchet
            );
        }
        for kind in [
            DmTypeId::Unfriended,
            DmTypeId::UnfriendedAck,
            DmTypeId::CallEnd,
            DmTypeId::SessionResetRequest,
            DmTypeId::SessionResetAccept,
            DmTypeId::FriendRequest,
            DmTypeId::FriendAccept,
            DmTypeId::PreKeyInit,
        ] {
            assert!(!on_chain(kind), "{kind:?} stays off the dm chain");
        }
    }
}
