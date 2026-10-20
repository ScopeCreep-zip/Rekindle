//! A first contact (plan E2.4, B2/B6/B7): the bundle a friend's prekey
//! record gives, the session it opens carrying our accept (or our reset
//! accept), the visible failure when it cannot, and the user's Retry.
//! Every path that opens a session with a friend from their record comes
//! here: an accept, a crossing request, a reset accept, B6's resend, a
//! send with no session, and a friend's re-sent request.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};
use rekindle_crypto::signal::PreKeyBundle;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::FirstContact;
use crate::operations::dm;
use crate::operations::friend::{state, FriendError, FriendHost};

/// The bundle a session with `peer` (an accepted friend) opens from: a
/// slot of the prekey record their profile names.
///
/// # Errors
/// Their profile is not known, or no bundle can be picked from it.
pub(crate) async fn bundle_for_friend(
    host: &dyn FriendHost,
    peer: &str,
) -> Result<PreKeyBundle, FriendError> {
    let profile = state::friend(host, peer)
        .and_then(|f| f.dht_record_key)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            FriendError::Conflict(format!(
                "A secure session with {} cannot be opened: their profile is not known",
                state::label(host, peer)
            ))
        })?;
    super::pick_bundle(host, peer, &profile).await
}

/// Open a session with `peer` from their record, carrying `kind`
/// (`FriendAccept` or `SessionResetAccept`) as its first payload, and send
/// it.
///
/// # Errors
/// `kind` is no first contact, no bundle could be picked, or the session
/// could not be opened or sent.
pub(crate) async fn open_first_contact(
    host: &dyn FriendHost,
    peer: &str,
    kind: DmTypeId,
) -> Result<(), FriendError> {
    let first = match kind {
        DmTypeId::FriendAccept => None,
        DmTypeId::SessionResetAccept => Some(DirectBody::SessionResetAccept),
        other => {
            return Err(FriendError::Invalid(format!(
                "{other:?} is no first contact"
            )))
        }
    };
    let bundle = bundle_for_friend(host, peer).await?;
    match first {
        None => dm::send_friend_accept(host, peer, bundle).await,
        Some(body) => {
            let frame = dm::frame::initiate_frame(host, peer, bundle, &body).await?;
            dm::send_frame(host, peer, &frame).await
        }
    }
}

/// [`open_first_contact`], its failure shown on the friend's row with
/// Retry ([`failed`]); whether it went out.
pub(crate) async fn open_or_fail(host: &dyn FriendHost, peer: &str, kind: DmTypeId) -> bool {
    match open_first_contact(host, peer, kind).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(peer = %peer, ?kind, error = %e, "first contact not sent");
            failed(host, peer, kind, e.to_string());
            false
        }
    }
}

/// B7: the first contact carrying `kind` cannot be completed. The friend's
/// row says why and offers Retry ([`retry`]), which re-sends `kind`. There
/// is no session-reset prompt for a first contact.
pub(crate) fn failed(host: &dyn FriendHost, peer: &str, kind: DmTypeId, reason: String) {
    host.stores().first_contacts.lock().insert(
        peer.to_string(),
        FirstContact {
            kind,
            retried: false,
            failed: true,
        },
    );
    state::update(host, peer, |f| {
        f.handshake_retrying = false;
        f.handshake_error = Some(reason.clone());
    });
    host.publish(SubscriptionEvent::Friend(FriendEvent::HandshakeFailed {
        peer_key: peer.to_string(),
        reason,
    }));
}

/// The user's Retry on a friend whose secure session did not open: the
/// pending first contact (our accept, or our reset accept when that was
/// the one that failed) again, from a fresh slot of their record, with
/// B6's one resend available again.
///
/// # Errors
/// They are no accepted friend, or the first contact failed again (the row
/// shows it).
pub async fn retry(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    if !state::is_accepted(host, peer) {
        return Err(FriendError::Conflict(
            "Retry is only for a friend whose secure session did not open".to_string(),
        ));
    }
    let kind = host
        .stores()
        .first_contacts
        .lock()
        .get(peer)
        .map_or(DmTypeId::FriendAccept, |c| c.kind);
    host.stores().first_contacts.lock().insert(
        peer.to_string(),
        FirstContact {
            kind,
            retried: false,
            failed: false,
        },
    );
    state::update(host, peer, |f| {
        f.handshake_error = None;
        f.handshake_retrying = true;
    });
    host.publish(SubscriptionEvent::Friend(FriendEvent::HandshakeRetrying {
        peer_key: peer.to_string(),
    }));
    tracing::info!(peer = %peer, ?kind, "first contact retried by the user");
    if let Err(e) = open_first_contact(host, peer, kind).await {
        failed(host, peer, kind, e.to_string());
        return Err(e);
    }
    if kind == DmTypeId::SessionResetAccept {
        // The reset the user accepted is answered now.
        host.stores().incoming_resets.lock().remove(peer);
    }
    Ok(())
}
