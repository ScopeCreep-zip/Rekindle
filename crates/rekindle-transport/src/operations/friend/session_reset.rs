//! Secure-session resets (P3.3) with their out-of-band safety number (plan
//! E2.3: libsignal's 60-digit format, `verification::safety_number`).
//!
//! The user starts one ([`start`]): a sealed request carrying a fresh
//! handout bundle. A friend's request is held for the user's review by the
//! backend ([`on_request`], plan E2.1), and the user accepts ([`accept`],
//! the PreKeyInit that opens a new session from their bundle and carries
//! `SessionResetAccept`) or declines ([`decline`], sealed). Only a
//! PreKeyInit naming the keys our request offered opens a session
//! ([`answers_reset`], checked before the init consumes them; plan C7.25).

use rekindle_codec::capnp_codec::direct::{
    DirectBody, PqPrekey, PreKeyInitBody, SessionResetDecline, SessionResetRequest,
};
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::{BackendHold, FriendHost};
use super::state;
use crate::operations::dm;

/// A session reset we requested from a peer: the keys our request's
/// handout bundle offered. Their PreKeyInit must name exactly these, or it
/// was not an answer to our request (plan C7.25).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutgoingReset {
    /// The signed prekey's id.
    pub signed_prekey: u32,
    /// The X25519 one-time prekey's id, if the bundle carried one.
    pub one_time_prekey: Option<u32>,
    /// The ML-KEM one-time key's id, if the bundle carried one.
    pub pq_one_time: Option<u32>,
}

impl OutgoingReset {
    /// Whether a PreKeyInit's prekey ids answer this request.
    #[must_use]
    pub fn answered_by(&self, init: &PreKeyInitBody) -> bool {
        let pq_one_time = match init.pq_prekey {
            PqPrekey::OneTime(id) => Some(id),
            PqPrekey::LastResort(_) => None,
        };
        self.signed_prekey == init.signed_prekey_id
            && self.one_time_prekey == init.one_time_prekey_id
            && self.pq_one_time == pq_one_time
    }
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

/// Reset our secure session with a friend: our request carries a fresh
/// handout bundle whose keys only their init may name. The old session
/// stays until theirs replaces it, so messages in flight still open.
///
/// # Errors
/// They are no friend, or the request could not be sent.
pub async fn start(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    if !state::is_accepted(host, peer) {
        return Err(FriendError::Conflict(
            "can only reset sessions with existing friends".to_string(),
        ));
    }
    let bundle = rekindle_db::prekeys::handout_bundle(&host.db()?, &host.local_party()?)
        .await
        .map_err(|e| FriendError::Failed(format!("handout bundle: {e}")))?;
    host.stores().outgoing_resets.lock().insert(
        peer.to_string(),
        OutgoingReset {
            signed_prekey: rekindle_crypto::signal::SPK_ID,
            one_time_prekey: bundle.one_time_prekey_id,
            pq_one_time: bundle.pqpk_ot_id,
        },
    );
    tracing::info!(peer = %peer, "session reset by user; sending SessionResetRequest");
    let request = DirectBody::SessionResetRequest(SessionResetRequest {
        prekey_bundle: bundle.to_wire(),
    });
    if let Err(e) = dm::send_to_peer(host, peer, &request).await {
        host.stores().outgoing_resets.lock().remove(peer);
        return Err(FriendError::Failed(format!(
            "send SessionResetRequest: {e}"
        )));
    }
    Ok(())
}

/// Accept the reset `peer` asked for: a new session from their bundle,
/// carrying `SessionResetAccept`. When its review was held on their
/// verified key changing (`VerifiedKeyChanged`), committing it was the
/// user's acknowledgement (`acknowledges_key_change`): the change is
/// acknowledged once the new session is recorded, since that record is
/// `rekindle_db::ratchet`'s own transaction.
///
/// # Errors
/// `NotFound` when no reset from them is pending; the session, the
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
    let bundle_bytes = host
        .stores()
        .incoming_resets
        .lock()
        .remove(peer)
        .ok_or_else(|| {
            FriendError::NotFound(
                "no pending session reset for this peer (it may have expired or already been \
                 processed)"
                    .to_string(),
            )
        })?;
    let bundle = rekindle_crypto::signal::PreKeyBundle::from_wire(&bundle_bytes)
        .map_err(|e| FriendError::Failed(format!("invalid bundle in pending reset: {e}")))?;
    let frame =
        dm::frame::initiate_frame(host, peer, bundle, &DirectBody::SessionResetAccept).await?;
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

/// A friend asked to reset our session: their bundle is kept (the newest
/// wins) and the backend holds the accept for the user's review. A bundle
/// under another identity than the sender's is never offered: the safety
/// number the user compares names the sender.
pub(crate) async fn on_request(host: &dyn FriendHost, sender: &str, bundle: &[u8]) {
    let under_sender = rekindle_crypto::signal::PreKeyBundle::from_wire(bundle)
        .ok()
        .is_some_and(|b| hex::encode(b.identity_key) == sender);
    if !under_sender {
        tracing::warn!(from = %sender, "SessionResetRequest without the sender's own bundle — dropping");
        return;
    }
    host.stores()
        .incoming_resets
        .lock()
        .insert(sender.to_string(), bundle.to_vec());
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

/// Whether `sender`'s PreKeyInit carrying `SessionResetAccept` answers our
/// outstanding request with the keys we offered; the request is consumed
/// when it does, so a repeat opens nothing.
pub(crate) fn answers_reset(host: &dyn FriendHost, sender: &str, init: &PreKeyInitBody) -> bool {
    let mut outgoing = host.stores().outgoing_resets.lock();
    match outgoing.get(sender) {
        Some(sent) if sent.answered_by(init) => {
            outgoing.remove(sender);
            true
        }
        Some(_) => {
            tracing::warn!(from = %sender,
                "dropping SessionResetAccept that names keys our request did not offer");
            false
        }
        None => {
            tracing::warn!(from = %sender, "dropping SessionResetAccept: we requested no session reset");
            false
        }
    }
}

/// The friend's new session is open (their PreKeyInit was ours to accept).
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

#[cfg(test)]
mod tests {
    use rekindle_codec::capnp_codec::direct::{
        DmTypeId, PqPrekey, PreKeyInitBody, RatchetBody, RatchetHeader,
    };

    use super::OutgoingReset;

    fn init(spk: u32, otpk: Option<u32>, pq: PqPrekey) -> PreKeyInitBody {
        PreKeyInitBody {
            base_key: [1; 32],
            signed_prekey_id: spk,
            one_time_prekey_id: otpk,
            pq_prekey: pq,
            kem_ciphertext: Vec::new(),
            first_type_id: DmTypeId::SessionResetAccept,
            message: RatchetBody {
                header: RatchetHeader {
                    dh: [2; 32],
                    pn: 0,
                    n: 0,
                },
                ciphertext: Vec::new(),
            },
        }
    }

    #[test]
    fn only_the_offered_keys_answer_a_reset() {
        let sent = OutgoingReset {
            signed_prekey: 1,
            one_time_prekey: Some(5),
            pq_one_time: Some(6),
        };
        assert!(sent.answered_by(&init(1, Some(5), PqPrekey::OneTime(6))));
        assert!(!sent.answered_by(&init(1, Some(5), PqPrekey::LastResort(6))));
        assert!(!sent.answered_by(&init(1, Some(9), PqPrekey::OneTime(6))));
        assert!(!sent.answered_by(&init(2, Some(5), PqPrekey::OneTime(6))));
    }
}
