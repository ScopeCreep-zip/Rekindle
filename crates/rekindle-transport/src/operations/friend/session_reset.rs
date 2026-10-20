//! Secure-session resets (P3.3) with their out-of-band safety number (plan
//! E2.3: libsignal's 60-digit format, `verification::safety_number`).
//!
//! The user starts one ([`start`]): a sealed request naming no keys (plan
//! E2.4: our prekey record is where the friend reads them). A friend's
//! request is held for the user's review by the backend ([`on_request`],
//! plan E2.1), and the user accepts ([`accept`], the PreKeyInit that opens
//! a new session from a slot of their record and carries
//! `SessionResetAccept`) or declines ([`decline`], sealed). Only a friend
//! we asked opens a session this way ([`awaits_accept`], B11): the ids its
//! init names are ours, and the init's own consumption checks them (B8).

use rekindle_codec::capnp_codec::direct::{DirectBody, SessionResetDecline, SessionResetRequest};
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::{BackendHold, FriendHost};
use super::state;
use crate::operations::dm;

/// A session reset we requested from a peer, outstanding until their
/// accept opens the new session or they decline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutgoingReset {
    /// When we asked, ms since the epoch.
    pub requested_at: i64,
}

/// A friend's reset request, held for the user's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingReset {
    /// Why they asked, as they put it (empty when they gave no reason).
    pub reason: String,
}

/// The safety number of the reset `peer` asked for, while it is pending.
#[must_use]
pub fn pending_safety_number(host: &dyn FriendHost, peer: &str) -> Option<String> {
    host.stores()
        .incoming_resets
        .lock()
        .contains_key(peer)
        .then_some(())?;
    super::verification::safety_number(host, peer)
}

fn alert(host: &dyn FriendHost, title: &str, body: String) {
    host.publish(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: title.to_string(),
            body,
        },
    ));
}

/// Reset our secure session with a friend: the request names no keys, and
/// only they may answer it, from our prekey record. The old session stays
/// until theirs replaces it, so messages in flight still open.
///
/// # Errors
/// They are no friend, or the request could not be sent.
pub async fn start(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    if !state::is_accepted(host, peer) {
        return Err(FriendError::Conflict(
            "can only reset sessions with existing friends".to_string(),
        ));
    }
    host.stores().outgoing_resets.lock().insert(
        peer.to_string(),
        OutgoingReset {
            requested_at: host.clock().now_ms_i64(),
        },
    );
    tracing::info!(peer = %peer, "session reset by user; sending SessionResetRequest");
    let request = DirectBody::SessionResetRequest(SessionResetRequest {
        reason: String::new(),
    });
    if let Err(e) = dm::send_to_peer(host, peer, &request).await {
        host.stores().outgoing_resets.lock().remove(peer);
        return Err(FriendError::Failed(format!(
            "send SessionResetRequest: {e}"
        )));
    }
    Ok(())
}

/// Accept the reset `peer` asked for: a new session from a slot of their
/// prekey record, carrying `SessionResetAccept`. Their request is answered
/// only once that session is opened; until then it stays pending. When its review was held on their
/// verified key changing (`VerifiedKeyChanged`), committing it was the
/// user's acknowledgement (`acknowledges_key_change`): the change is
/// acknowledged once the new session is recorded, since that record is
/// `rekindle_db::ratchet`'s own transaction.
///
/// # Errors
/// `NotFound` when no reset from them is pending; their record gave no
/// bundle (the request stays pending), or the session, the
/// acknowledgement or the send failed.
pub async fn accept(
    host: &dyn FriendHost,
    peer: &str,
    acknowledges_key_change: bool,
) -> Result<(), FriendError> {
    if !state::is_accepted(host, peer) {
        return Err(FriendError::Conflict(
            "can only accept session reset from existing friends".to_string(),
        ));
    }
    if !host.stores().incoming_resets.lock().contains_key(peer) {
        return Err(FriendError::NotFound(
            "no pending session reset for this peer (it may have expired or already been \
             processed)"
                .to_string(),
        ));
    }
    let profile = state::friend(host, peer)
        .and_then(|f| f.dht_record_key)
        .ok_or_else(|| FriendError::Conflict(format!("{peer}'s profile is not known")))?;
    let bundle = crate::operations::prekeys::pick_bundle(host, peer, &profile).await?;
    let frame =
        dm::frame::initiate_frame(host, peer, bundle, &DirectBody::SessionResetAccept).await?;
    // Answered only once the session is opened: a record that cannot be
    // read, or is not theirs (B3), leaves the request for the user to
    // accept again or decline.
    host.stores().incoming_resets.lock().remove(peer);
    if acknowledges_key_change {
        super::verification::acknowledge_key_change(host, peer).await?;
    }
    dm::send_frame(host, peer, &frame)
        .await
        .map_err(|e| FriendError::Failed(format!("send SessionResetAccept: {e}")))?;
    tracing::info!(peer = %peer, "session reset accepted; the new session's init is out");
    Ok(())
}

/// Decline the reset `peer` asked for and tell them.
///
/// # Errors
/// The decline could not be sent.
pub async fn decline(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    host.stores().incoming_resets.lock().remove(peer);
    let decline = DirectBody::SessionResetDecline(SessionResetDecline {
        reason: String::new(),
    });
    dm::send_to_peer(host, peer, &decline).await
}

/// A friend asked to reset our session: the request is kept (the newest
/// wins) and the backend holds the accept for the user's review.
pub(crate) async fn on_request(host: &dyn FriendHost, sender: &str, reason: &str) {
    host.stores().incoming_resets.lock().insert(
        sender.to_string(),
        IncomingReset {
            reason: reason.to_string(),
        },
    );
    if let Err(e) = host
        .hold_backend_intent(BackendHold::SessionReset {
            peer: sender.to_string(),
        })
        .await
    {
        host.stores().incoming_resets.lock().remove(sender);
        tracing::warn!(from = %sender, error = %e, "SessionResetRequest not held — dropped");
    }
}

/// Whether `sender` may open a session with `SessionResetAccept`: a reset
/// we asked them for is outstanding (B11). It is consumed when the accept
/// is applied ([`on_accept`]).
pub(crate) fn awaits_accept(host: &dyn FriendHost, sender: &str) -> bool {
    let outstanding = host.stores().outgoing_resets.lock().contains_key(sender);
    if !outstanding {
        tracing::warn!(from = %sender, "dropping SessionResetAccept: we requested no session reset");
    }
    outstanding
}

/// The friend's new session is open (their PreKeyInit answered our
/// request, which is done).
pub(crate) fn on_accept(host: &dyn FriendHost, sender: &str) {
    host.stores().outgoing_resets.lock().remove(sender);
    let display = state::label(host, sender);
    alert(
        host,
        "Secure session re-established",
        format!(
            "A new secure session with {display} is active. Verify their safety number \
             out-of-band before resuming sensitive conversations."
        ),
    );
}

/// The friend declined our reset.
pub(crate) fn on_decline(host: &dyn FriendHost, sender: &str, reason: &str) {
    host.stores().outgoing_resets.lock().remove(sender);
    let display = state::label(host, sender);
    let body = if reason.is_empty() {
        format!("{display} declined your secure session reset request.")
    } else {
        format!("{display} declined your secure session reset request: {reason}")
    };
    alert(host, "Session Reset Declined", body);
}
