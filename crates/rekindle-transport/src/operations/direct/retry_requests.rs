//! Retry requests (plan E2.5, D24): both halves of `RetryRequest` (0x0006).
//!
//! Receiver: a user-content ratchet message that does not open is held
//! ([`note_failed`]) and asked for only after the source's CaughtUp
//! ([`on_caught_up`]), within the caps of `rekindle_db::repo::
//! retry_requests` (5 per message, 10 per peer per 3 h), each request
//! after a jittered delay from the RNG port.
//!
//! Sender: a request resends the logged message under the session we hold
//! now, and only when the ratchet key it names is the one we logged
//! ([`on_retry_request`]). A request naming another key changes no
//! session and resends nothing; nothing here ever re-initiates a session
//! (threat model, Scope: no auto-rehandshake).
//!
//! Exhausted: a message that fails again after its last request
//! (`Taken::exhausted`) makes the Host hold a `SessionReinitiate` review
//! for the user (ruling Q1); only their commit runs the reset.

use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{
    DirectBody, DmFrame, DmTypeId, FrameBody, RatchetBody, RetryRequest,
};
use rekindle_crypto::replay::ratchet_message_id;
use rekindle_db::ratchet::{self, RatchetError};
use rekindle_db::repo::{retry_requests, send_log};

use crate::operations::dm::{self, frame};
use crate::operations::friend::{state, BackendHold, FriendHost};

/// The most a request waits before it goes, so a burst of failures after
/// a catch-up does not leave in one instant.
pub const RETRY_JITTER_MAX: Duration = Duration::from_secs(5);

fn peer_key(peer: &str) -> Option<[u8; 32]> {
    <[u8; 32]>::try_from(hex::decode(peer).ok()?).ok()
}

/// Whether a failed open of `type_id` is worth a request: a user-content
/// kind ([`frame::is_logged`]) under a session we hold (with none, no
/// ratchet frame can carry the request; that is ruling Q1's prompt), and
/// not a copy of one already opened.
#[must_use]
pub fn requestable(type_id: DmTypeId, error: &RatchetError) -> bool {
    use rekindle_crypto::CryptoError;
    frame::is_logged(type_id)
        && !matches!(
            error,
            RatchetError::Crypto(CryptoError::DuplicateMessage | CryptoError::NoSession(_))
        )
}

/// A user-content ratchet message from `sender` did not open: hold a
/// request for it until their CaughtUp.
pub(super) async fn note_failed(host: &dyn FriendHost, sender: [u8; 32], body: &RatchetBody) {
    let (Ok(owner), Ok(db)) = (host.local_party().map(|l| l.owner()), host.db()) else {
        return;
    };
    let due = retry_requests::Due {
        msg_id: ratchet_message_id(&body.header.dh, body.header.n),
        observed_ratchet_key: body.header.dh,
    };
    let now = host.clock().now_ms_i64();
    if let Err(e) = db
        .call(move |conn| retry_requests::note_failed(conn, &owner, &sender, &due, now))
        .await
    {
        tracing::warn!(error = %e, "retry request not recorded");
    }
}

/// `peer` (a hex identity) has caught up: send the requests the caps
/// allow, each after a jittered delay, and hold the user's review of a
/// session re-initiation when a message's requests are exhausted
/// ([`hold_reinitiate`]). The sync engine's CaughtUp calls this (D23);
/// nothing else does.
pub async fn on_caught_up(host: &dyn FriendHost, peer: &str) {
    let (Some(peer_bytes), Ok(owner), Ok(db)) = (
        peer_key(peer),
        host.local_party().map(|l| l.owner()),
        host.db(),
    ) else {
        return;
    };
    let now = host.clock().now_ms_i64();
    let taken = match db
        .call(move |conn| retry_requests::take_due(conn, &owner, &peer_bytes, now))
        .await
    {
        Ok(taken) => taken,
        Err(e) => {
            tracing::warn!(error = %e, "retry requests unreadable");
            return;
        }
    };
    if taken.exhausted > 0 {
        hold_reinitiate(host, peer, taken.exhausted).await;
    }
    let jitter_ms = u64::try_from(RETRY_JITTER_MAX.as_millis()).expect("seconds fit u64 ms");
    for d in taken.due {
        let delay = Duration::from_millis(u64::from(host.rng().next_u32()) % (jitter_ms + 1));
        let (owned, peer) = (host.owned(), peer.to_string());
        host.scope().spawn_or_drop("retry request", async move {
            tokio::time::sleep(delay).await;
            let body = DirectBody::RetryRequest(RetryRequest {
                msg_id: d.msg_id,
                observed_ratchet_key: d.observed_ratchet_key,
                ts: owned.clock().now_ms(),
            });
            if let Err(e) = dm::send_to_peer(owned.as_ref(), &peer, &body).await {
                tracing::warn!(to = %peer, error = %e, "retry request not sent");
            }
        });
    }
}

/// `peer`'s retry requests are exhausted: hold a `SessionReinitiate`
/// review for the user (ruling Q1). It sends nothing; one hold per friend
/// (a newer replaces the older), and it expires like every backend hold.
pub async fn hold_reinitiate(host: &dyn FriendHost, peer: &str, exhausted: usize) {
    if !state::is_accepted(host, peer) {
        return;
    }
    tracing::info!(peer = %peer, exhausted, "retry requests exhausted; re-initiation held for review");
    let held = BackendHold::SessionReinitiate {
        peer: peer.to_string(),
    };
    if let Err(e) = host.hold_backend_intent(held).await {
        tracing::warn!(peer = %peer, error = %e, "session re-initiation not held");
    }
}

/// A `RetryRequest` from `sender`: resend the logged message under our
/// current session when the key it names is the logged one. Whether it
/// was handled (a request matching nothing is handled: there is nothing
/// to do).
pub(super) async fn on_retry_request(
    host: &dyn FriendHost,
    sender: &str,
    request: &RetryRequest,
) -> bool {
    let (Ok(address), Ok(local), Ok(db)) =
        (frame::peer_address(sender), host.local_party(), host.db())
    else {
        return false;
    };
    let (owner, peer, now, req) = (
        local.owner(),
        address.identity,
        local.now_ms,
        request.clone(),
    );
    let logged = match db
        .call(move |conn| {
            send_log::take_for_resend(
                conn,
                &owner,
                &peer,
                &req.msg_id,
                &req.observed_ratchet_key,
                now,
            )
        })
        .await
    {
        Ok(Some(logged)) => logged,
        Ok(None) => {
            tracing::debug!(from = %sender, "retry request names no logged message under its key");
            return true;
        }
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "send log unreadable");
            return false;
        }
    };
    let Some(type_id) = DmTypeId::from_u16(logged.type_id) else {
        return true;
    };
    let message = match ratchet::encrypt_record_and_log_outbound(
        &db,
        &local,
        &address,
        &logged.plaintext,
        &frame::ad_extra(type_id),
        logged.type_id,
    )
    .await
    {
        Ok(message) => message,
        Err(e) => {
            tracing::warn!(to = %sender, error = %e, "resend refused: no current session");
            return true;
        }
    };
    let resent = DmFrame {
        type_id,
        sender: local.owner(),
        body: FrameBody::Ratchet(frame::ratchet_body(message)),
    };
    if let Err(e) = dm::send_frame(host, sender, &resent).await {
        tracing::warn!(to = %sender, error = %e, "resend not delivered");
    }
    true
}

/// Drop the send-log rows past `DELIVERY_HORIZON` and the retry state past
/// its windows (a Host unlock stage, with the replay).
pub(super) async fn prune(host: &dyn FriendHost) {
    let (Ok(owner), Ok(db)) = (host.local_party().map(|l| l.owner()), host.db()) else {
        return;
    };
    let now = host.clock().now_ms_i64();
    if let Err(e) = db
        .call(move |conn| {
            send_log::prune(conn, &owner, now)?;
            retry_requests::prune(conn, &owner, now)
        })
        .await
    {
        tracing::warn!(error = %e, "send log not pruned");
    }
}

#[cfg(test)]
mod tests {
    use rekindle_crypto::CryptoError;

    use super::*;

    #[test]
    fn only_user_content_under_a_session_is_requested() {
        let failed = RatchetError::Crypto(CryptoError::SessionError("aead".into()));
        assert!(requestable(DmTypeId::DmText, &failed));
        assert!(requestable(DmTypeId::DmReaction, &failed));
        // Not user content: nothing was logged to resend.
        assert!(!requestable(DmTypeId::Typing, &failed));
        assert!(!requestable(DmTypeId::RetryRequest, &failed));
        assert!(!requestable(DmTypeId::FriendsKey, &failed));
        // A copy of one already opened, or no session to carry a request.
        let dup = RatchetError::Crypto(CryptoError::DuplicateMessage);
        assert!(!requestable(DmTypeId::DmText, &dup));
        let none = RatchetError::Crypto(CryptoError::NoSession("peer".into()));
        assert!(!requestable(DmTypeId::DmText, &none));
    }

    #[test]
    fn the_jitter_is_bounded() {
        assert!(RETRY_JITTER_MAX <= Duration::from_secs(5));
    }
}
