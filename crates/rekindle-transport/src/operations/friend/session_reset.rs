//! Secure-session resets (P3.3) with their out-of-band safety number (plan
//! E2.3: libsignal's 60-digit format, `verification::safety_number`).
//!
//! The user starts one ([`start`]): a sealed request naming no keys (plan
//! E2.4: our prekey record is where the friend reads them). A friend's
//! request is held for the user's review by the backend ([`on_request`],
//! plan E2.1), and the user accepts ([`accept`], the PreKeyInit that opens
//! a new session from a slot of their record and carries
//! `SessionResetAccept`) or declines ([`decline`], sealed). Only a friend
//! we asked opens a session this way (B11): our request is stored
//! (`session_reset_requests`) and consumed in the transaction that accepts
//! their init (`ratchet::accept_init`, plan audit D-4); the ids its init
//! names are ours, and the init's own consumption checks them (B8).

use rekindle_codec::capnp_codec::direct::{
    DirectBody, DmTypeId, SessionResetDecline, SessionResetRequest,
};
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::{BackendHold, FriendHost};
use super::state;
use crate::operations::dm;

/// A friend's reset request, held for the user's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingReset {
    /// Why they asked, as they put it (empty when they gave no reason).
    pub reason: String,
    /// When it arrived, ms since the epoch: it expires with the backend
    /// hold that waits on it ([`expire_resets`]).
    pub requested_at: i64,
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
    let owner = state::owner(host)?;
    let (ok, pk, now) = (owner.clone(), peer.to_string(), host.clock().now_ms_i64());
    state::db_call!(host, "store the reset request", move |conn| {
        rekindle_db::repo::session_reset_requests::insert(conn, &ok, &pk, now)
    })
    .await?;
    tracing::info!(peer = %peer, "session reset by user; sending SessionResetRequest");
    let request = DirectBody::SessionResetRequest(SessionResetRequest {
        reason: String::new(),
    });
    if let Err(e) = dm::send_to_peer(host, peer, &request).await {
        let pk = peer.to_string();
        state::db_fire!(host, "withdraw the reset request", move |conn| {
            rekindle_db::repo::session_reset_requests::take(conn, &owner, &pk).map(|_| ())
        });
        return Err(FriendError::Failed(format!(
            "send SessionResetRequest: {e}"
        )));
    }
    Ok(())
}

/// Accept the reset `peer` asked for: a new session from a slot of their
/// prekey record, carrying `SessionResetAccept`. Their request is answered
/// only once that session is opened and sent; until then it stays pending. When its review was held on their
/// verified key changing (`VerifiedKeyChanged`), committing it was the
/// user's acknowledgement (`acknowledges_key_change`): the change is
/// acknowledged once the new session is recorded, since that record is
/// `rekindle_db::ratchet`'s own transaction.
///
/// # Errors
/// `NotFound` when no reset from them is pending; their record gave no
/// bundle, or the session or the send failed (the request stays pending,
/// and their row offers Retry), or the acknowledgement failed.
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
    if !acknowledges_key_change {
        super::verification::require_acknowledged(host, peer).await?;
    }
    let kind = DmTypeId::SessionResetAccept;
    if let Err(e) = crate::operations::prekeys::open_first_contact(host, peer, kind).await {
        // Not answered: a record that cannot be read, or is not theirs
        // (B3), or a send that failed shows on their row with Retry (B7),
        // which re-sends this reset accept; the request stays until then.
        crate::operations::prekeys::failed(host, peer, kind, e.to_string());
        return Err(e);
    }
    host.stores().incoming_resets.lock().remove(peer);
    if acknowledges_key_change {
        super::verification::acknowledge_key_change(host, peer).await?;
    }
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
/// wins) and the backend holds the accept for the user's review; whether
/// the hold landed.
pub(crate) async fn on_request(host: &dyn FriendHost, sender: &str, reason: &str) -> bool {
    host.stores().incoming_resets.lock().insert(
        sender.to_string(),
        IncomingReset {
            reason: reason.to_string(),
            requested_at: host.clock().now_ms_i64(),
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
        return false;
    }
    true
}

/// The friend's new session is open (their PreKeyInit answered our
/// request, consumed in the transaction that accepted it).
pub(crate) fn on_accept(host: &dyn FriendHost, sender: &str) {
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

/// The friend declined our reset. Their reason is peer text: it never goes
/// into an alert, which the OS shows (plan audit F-4).
pub(crate) fn on_decline(host: &dyn FriendHost, sender: &str) {
    if let Ok(owner) = state::owner(host) {
        let pk = sender.to_string();
        state::db_fire!(host, "delete the declined reset request", move |conn| {
            rekindle_db::repo::session_reset_requests::take(conn, &owner, &pk).map(|_| ())
        });
    }
    let display = state::label(host, sender);
    alert(
        host,
        "Session Reset Declined",
        format!("{display} declined your secure session reset request."),
    );
}

/// Expire the reset requests older than `ttl_ms` (the backend-hold
/// lifetime, plan audit D-4): ours unanswered, and the friends' the user
/// never answered (their hold expired with them).
///
/// # Errors
/// Our stored requests could not be expired.
pub async fn expire_resets(host: &dyn FriendHost, ttl_ms: u64) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let cutoff = host
        .clock()
        .now_ms_i64()
        .saturating_sub(i64::try_from(ttl_ms).unwrap_or(i64::MAX));
    host.stores()
        .incoming_resets
        .lock()
        .retain(|_, reset| reset.requested_at >= cutoff);
    let expired = state::db_call!(host, "expire reset requests", move |conn| {
        rekindle_db::repo::session_reset_requests::delete_before(conn, &owner, cutoff)
    })
    .await?;
    if expired > 0 {
        tracing::info!(expired, "expired unanswered session reset requests");
    }
    Ok(())
}
