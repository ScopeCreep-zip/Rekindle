//! Safety numbers and verification (plan E2.3): our safety number with a
//! peer (libsignal's 60 digits, `rekindle_crypto::SafetyNumber`), its
//! scannable form as a QR code, and the user's mark that they compared it
//! out of band (`signal_identities.verified`).
//!
//! Under identity == address a peer's key is their public key, so the
//! number is computed from the two identities. The peer's row is recorded
//! with the first session (trust on first use, unverified); a rotation
//! moves it, clears the mark and, when it was verified, records the change
//! until an escalated action acknowledges it (`VerifiedKeyChanged`).

use rekindle_crypto::SafetyNumber;
use rekindle_types::dto::friends::SafetyNumberView;
use rekindle_types::intent::Escalation;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// Our identity key and `peer`'s, as `signal_identities` keys them (and
/// an action that acknowledges a key change in its own transaction,
/// `signal_identities::acknowledging`).
///
/// # Errors
/// `Locked` while no identity is unlocked; `Invalid` when `peer` is not a
/// public key.
pub(crate) fn keys(host: &dyn FriendHost, peer: &str) -> Result<([u8; 32], [u8; 32]), FriendError> {
    let parse = crate::operations::dm::recipient_key;
    Ok((parse(&state::owner(host)?)?, parse(peer)?))
}

/// The safety number of our identity and `peer`'s, as both sides display
/// it.
#[must_use]
pub fn safety_number(host: &dyn FriendHost, peer: &str) -> Option<String> {
    let (owner, theirs) = keys(host, peer).ok()?;
    Some(SafetyNumber::new(&owner, &theirs).digits())
}

/// Our safety number with `peer`, its QR, and whether we verified them.
///
/// # Errors
/// The identity is locked, `peer` is not a key, or the row could not be
/// read.
pub async fn view(host: &dyn FriendHost, peer: &str) -> Result<SafetyNumberView, FriendError> {
    let (owner, theirs) = keys(host, peer)?;
    let row = state::db_call!(host, "read verification", move |conn| {
        rekindle_db::repo::signal_identities::get(conn, &owner, &theirs)
    })
    .await?;
    let number = SafetyNumber::new(&owner, &theirs);
    let scannable = number.scannable();
    Ok(SafetyNumberView {
        peer_key: peer.to_string(),
        digits: number.digits(),
        // The form is 74 bytes, which always fits a QR code.
        qr_rows: super::qr::rows(&scannable, "safety number")?,
        scannable,
        verified: row.as_ref().is_some_and(|r| r.verified),
        verified_at: row.as_ref().and_then(|r| r.verified_at),
        changed: row.is_some_and(|r| r.changed_from_key.is_some()),
    })
}

/// The user compared the safety number with `peer` and marks their key
/// verified (an unacknowledged key change is acknowledged with it).
///
/// # Errors
/// `NotFound` when we hold no session row for `peer` yet (it is recorded
/// with the first session); the write failed.
pub async fn mark_verified(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    let (owner, theirs) = keys(host, peer)?;
    let now = host.clock().now_ms_i64();
    let marked = state::db_call!(host, "mark verified", move |conn| {
        rekindle_db::repo::signal_identities::mark_verified(conn, &owner, &theirs, now)
    })
    .await?;
    if !marked {
        return Err(no_session(peer));
    }
    tracing::info!(peer = %peer, "identity key marked verified");
    publish_verification(host, peer, true);
    Ok(())
}

/// The user withdraws `peer`'s verification.
///
/// # Errors
/// `NotFound` when we hold no session row for `peer`; the write failed.
pub async fn clear_verification(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    let (owner, theirs) = keys(host, peer)?;
    let cleared = state::db_call!(host, "clear verification", move |conn| {
        rekindle_db::repo::signal_identities::clear_verification(conn, &owner, &theirs)
    })
    .await?;
    if !cleared {
        return Err(no_session(peer));
    }
    tracing::info!(peer = %peer, "identity key verification cleared");
    publish_verification(host, peer, false);
    Ok(())
}

/// Tell every frontend that `peer`'s verification changed, so each window
/// showing their badge or safety number re-reads it.
fn publish_verification(host: &dyn FriendHost, peer: &str, verified: bool) {
    host.publish(SubscriptionEvent::Friend(
        FriendEvent::VerificationChanged {
            peer_key: peer.to_string(),
            verified,
        },
    ));
}

fn no_session(peer: &str) -> FriendError {
    FriendError::NotFound(format!(
        "no secure session with {peer} yet: their key is recorded with the first one"
    ))
}

/// `VerifiedKeyChanged` while the key we verified for `peer` changed and
/// the change is unacknowledged: the review of the next action toward them
/// is held on it.
///
/// # Errors
/// The identity is locked, `peer` is not a key, or the row could not be
/// read.
pub async fn key_change(
    host: &dyn FriendHost,
    peer: &str,
) -> Result<Option<Escalation>, FriendError> {
    let (owner, theirs) = keys(host, peer)?;
    state::db_call!(host, "read key change", move |conn| {
        rekindle_db::repo::signal_identities::verified_key_changed(conn, &owner, &theirs)
    })
    .await
}

/// Refuse any frame to `peer` while the key we verified for them changed
/// and the change is unacknowledged (plan audit R-4): a send whose review
/// was held on it acknowledges it before it is framed (`send_dm`,
/// `resend_dm`), or carries it (`session_reset::accept`); nothing else goes,
/// typing included.
///
/// # Errors
/// `Conflict` while the change is unacknowledged, or it could not be read.
/// Whether `kind` only withdraws from, or refuses, the peer: removing them,
/// rejecting their request, declining their reset, refusing their prekeys.
/// These go even while their key change is unacknowledged, so the user can
/// always back away from a changed key (threat model S1).
pub(crate) fn is_protective(kind: rekindle_codec::capnp_codec::direct::DmTypeId) -> bool {
    use rekindle_codec::capnp_codec::direct::DmTypeId;
    matches!(
        kind,
        DmTypeId::Unfriended
            | DmTypeId::UnfriendedAck
            | DmTypeId::FriendReject
            | DmTypeId::SessionResetDecline
            | DmTypeId::PreKeyRejected
    )
}

pub(crate) async fn require_acknowledged(
    host: &dyn FriendHost,
    peer: &str,
) -> Result<(), FriendError> {
    if key_change(host, peer).await?.is_some() {
        return Err(FriendError::Conflict(format!(
            "the verified key of {peer} changed: acknowledge the change before sending to them"
        )));
    }
    Ok(())
}

/// Acknowledge `peer`'s key change after an escalated action whose own
/// write is not ours to join (a session-reset accept, whose ratchet record
/// is `rekindle_db::ratchet`'s transaction). A crash before it leaves the
/// change unacknowledged, so the next action asks again.
///
/// # Errors
/// The write failed.
pub(crate) async fn acknowledge_key_change(
    host: &dyn FriendHost,
    peer: &str,
) -> Result<(), FriendError> {
    let (owner, theirs) = keys(host, peer)?;
    state::db_call!(host, "acknowledge key change", move |conn| {
        rekindle_db::repo::signal_identities::acknowledge_key_change(conn, &owner, &theirs)
    })
    .await
}

#[cfg(test)]
mod tests {
    /// Withdrawing and refusing go through an unacknowledged key change;
    /// content, typing and session openers do not.
    #[test]
    fn only_withdrawals_skip_the_key_change_hold() {
        use rekindle_codec::capnp_codec::direct::DmTypeId;
        for kind in [
            DmTypeId::Unfriended,
            DmTypeId::UnfriendedAck,
            DmTypeId::FriendReject,
            DmTypeId::SessionResetDecline,
            DmTypeId::PreKeyRejected,
        ] {
            assert!(super::is_protective(kind), "{kind:?}");
        }
        for kind in [
            DmTypeId::DmText,
            DmTypeId::Typing,
            DmTypeId::PreKeyInit,
            DmTypeId::CallInvite,
        ] {
            assert!(!super::is_protective(kind), "{kind:?}");
        }
    }
}
