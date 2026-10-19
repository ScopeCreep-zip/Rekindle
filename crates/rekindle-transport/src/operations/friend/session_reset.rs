//! Signal session resets (P3.3) with their out-of-band safety number.
//!
//! The user starts one ([`start`]); a friend's request is held for the
//! user's review by the backend ([`on_request`], plan E2.1), and the user
//! accepts ([`accept`]) or declines ([`decline`]) it. Only an accept naming
//! the keys our request offered installs a session (plan C7.25).

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::{BackendHold, FriendHost};
use super::state;
use crate::operations::dm;

/// A Signal session reset we requested from a peer: the keys our
/// request's handout bundle offered. Their accept must name exactly these,
/// or it was not an answer to our request (plan C7.25).
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
    /// Whether an accept's handshake ids answer this request.
    #[must_use]
    pub fn answered_by(&self, spk_id: u32, otpk_id: Option<u32>, pq_ot_id: Option<u32>) -> bool {
        self.signed_prekey == spk_id
            && self.one_time_prekey == otpk_id
            && self.pq_one_time == pq_ot_id
    }
}

/// The short out-of-band safety number for us and the identity in a peer's
/// bundle (BLAKE3 of the sorted Ed25519 identity keys, so both sides see the
/// same code).
#[must_use]
pub fn safety_number(host: &dyn FriendHost, peer_bundle: &[u8]) -> Option<String> {
    let bundle = rekindle_crypto::signal::PreKeyBundle::from_wire(peer_bundle).ok()?;
    let secret = host.identity_secret().ok()?;
    let ours = rekindle_crypto::Identity::from_secret_bytes(&secret).public_key_bytes();
    Some(rekindle_crypto::identity::safety_number(
        &ours,
        &bundle.identity_key,
    ))
}

/// The safety number of the reset `peer` asked for, while it is pending.
#[must_use]
pub fn pending_safety_number(host: &dyn FriendHost, peer: &str) -> Option<String> {
    let bundle = host.stores().incoming_resets.lock().get(peer).cloned()?;
    safety_number(host, &bundle)
}

fn alert(host: &dyn FriendHost, title: &str, body: String) {
    host.publish(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: title.to_string(),
            body,
        },
    ));
}

/// Reset our secure session with a friend: the old session goes, and our
/// request carries a fresh handout bundle whose keys only their accept may
/// name.
///
/// # Errors
/// They are no friend, or the request could not be sent.
pub async fn start(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    if !state::is_accepted(host, peer) {
        return Err(FriendError::Conflict(
            "can only reset sessions with existing friends".to_string(),
        ));
    }
    let signal = host.signal()?;
    signal
        .delete_session(peer)
        .map_err(|e| FriendError::Failed(format!("delete_session: {e}")))?;
    let bundle = signal
        .handout_bundle()
        .map_err(|e| FriendError::Failed(format!("handout_bundle: {e}")))?;
    host.stores().outgoing_resets.lock().insert(
        peer.to_string(),
        OutgoingReset {
            signed_prekey: rekindle_crypto::signal::SPK_ID,
            one_time_prekey: bundle.one_time_prekey_id,
            pq_one_time: bundle.pqpk_ot_id,
        },
    );
    tracing::info!(peer = %peer, "Signal session reset by user; sending SessionResetRequest");
    let payload = MessagePayload::SessionResetRequest {
        our_prekey_bundle: bundle.to_wire(),
    };
    if let Err(e) = dm::send_to_peer(host, peer, &payload).await {
        host.stores().outgoing_resets.lock().remove(peer);
        return Err(FriendError::Failed(format!(
            "send SessionResetRequest: {e}"
        )));
    }
    Ok(())
}

/// Accept the reset `peer` asked for: a fresh initiator session from their
/// bundle, and our accept naming its keys.
///
/// # Errors
/// `NotFound` when no reset from them is pending; the session or the
/// accept failed.
pub async fn accept(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
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
            "no pending session reset for this peer (it may have expired or already been processed)"
                .to_string(),
        )
        })?;
    let bundle = rekindle_crypto::signal::PreKeyBundle::from_wire(&bundle_bytes)
        .map_err(|e| FriendError::Failed(format!("invalid PreKeyBundle in pending reset: {e}")))?;
    let signal = host.signal()?;
    if let Err(e) = signal.delete_session(peer) {
        tracing::warn!(peer = %peer, error = %e, "old session not deleted before reset");
    }
    let init = signal
        .establish_session(peer, &bundle)
        .map_err(|e| FriendError::Failed(format!("establish_session: {e}")))?;
    let secret = host.identity_secret()?;
    let payload = MessagePayload::SessionResetAccept {
        ephemeral_key: init.ephemeral_public_key,
        signed_prekey_id: init.signed_prekey_id,
        one_time_prekey_id: init.one_time_prekey_id,
        our_identity_key: rekindle_crypto::Identity::from_secret_bytes(&secret)
            .public_key_bytes()
            .to_vec(),
        ml_kem_ciphertext: init.ml_kem_ciphertext,
        used_ot_pqpk_id: init.used_ot_pqpk_id,
    };
    dm::send_to_peer(host, peer, &payload)
        .await
        .map_err(|e| FriendError::Failed(format!("send SessionResetAccept: {e}")))?;
    tracing::info!(peer = %peer, "Signal session renewal accepted; SessionResetAccept dispatched");
    Ok(())
}

/// Decline the reset `peer` asked for and tell them.
///
/// # Errors
/// The decline could not be sent.
pub async fn decline(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    host.stores().incoming_resets.lock().remove(peer);
    let payload = MessagePayload::SessionResetDecline {
        reason: String::new(),
    };
    dm::send_to_peer(host, peer, &payload).await
}

/// A friend asked to reset our session: their bundle is kept (the newest
/// wins) and the backend holds the accept for the user's review. A request
/// with no computable safety number is never offered: comparing it out of
/// band is the user's only defence.
pub(crate) async fn on_request(host: &dyn FriendHost, sender: &str, bundle: &[u8]) {
    if !state::is_accepted(host, sender) {
        tracing::debug!(from = %sender, "dropping SessionResetRequest from non-friend");
        return;
    }
    if bundle.is_empty() {
        tracing::warn!(from = %sender, "SessionResetRequest with empty prekey bundle — dropping");
        return;
    }
    if safety_number(host, bundle).is_none() {
        tracing::warn!(from = %sender, "SessionResetRequest without a computable safety number — dropping");
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

/// The handshake ids of an incoming session-reset accept.
pub(crate) struct ResetAccept<'a> {
    pub ephemeral_key: &'a [u8],
    pub signed_prekey_id: u32,
    pub one_time_prekey_id: Option<u32>,
    pub identity_key: &'a [u8],
    pub ml_kem_ciphertext: &'a [u8],
    pub used_ot_pqpk_id: Option<u32>,
}

/// The friend accepted our reset: a fresh responder session, only when it
/// answers our outstanding request with the keys we offered (consumed, so a
/// repeat installs nothing).
pub(crate) fn on_accept(host: &dyn FriendHost, sender: &str, a: &ResetAccept<'_>) {
    if !state::is_accepted(host, sender) {
        tracing::debug!(from = %sender, "dropping SessionResetAccept from non-friend");
        return;
    }
    if a.ephemeral_key.is_empty() || a.identity_key.is_empty() {
        tracing::warn!(from = %sender, "SessionResetAccept missing ephemeral or identity key");
        return;
    }
    {
        let mut outgoing = host.stores().outgoing_resets.lock();
        match outgoing.remove(sender) {
            Some(sent)
                if sent.answered_by(
                    a.signed_prekey_id,
                    a.one_time_prekey_id,
                    a.used_ot_pqpk_id,
                ) => {}
            Some(sent) => {
                // Not an answer to our request; keep it for the real one.
                outgoing.insert(sender.to_string(), sent);
                tracing::warn!(from = %sender,
                    "dropping SessionResetAccept that names keys our request did not offer");
                return;
            }
            None => {
                tracing::warn!(from = %sender, "dropping SessionResetAccept: we requested no session reset");
                return;
            }
        }
    }
    let Ok(signal) = host.signal() else {
        tracing::warn!(from = %sender, "SessionResetAccept arrived but no Signal manager");
        return;
    };
    if let Err(e) = signal.delete_session(sender) {
        tracing::warn!(from = %sender, error = %e, "old session not deleted before reset");
    }
    match signal.respond_to_session(
        sender,
        a.identity_key,
        a.ephemeral_key,
        a.signed_prekey_id,
        a.one_time_prekey_id,
        a.ml_kem_ciphertext,
        a.used_ot_pqpk_id,
    ) {
        Ok(()) => {
            tracing::info!(from = %sender, "established responder Signal session from SessionResetAccept");
            let display = state::label(host, sender);
            alert(
                host,
                "Secure session re-established",
                format!(
                    "New Signal session with {display} is active. Verify their safety number \
                     out-of-band before resuming sensitive conversations."
                ),
            );
        }
        Err(error) => {
            tracing::warn!(from = %sender, %error, "failed to respond_to_session from SessionResetAccept");
        }
    }
}

/// The friend declined our reset.
pub(crate) fn on_decline(host: &dyn FriendHost, sender: &str, reason: &str) {
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
    use super::OutgoingReset;

    #[test]
    fn only_the_offered_keys_answer_a_reset() {
        let sent = OutgoingReset {
            signed_prekey: 1,
            one_time_prekey: Some(5),
            pq_one_time: Some(6),
        };
        assert!(sent.answered_by(1, Some(5), Some(6)));
        assert!(!sent.answered_by(1, Some(5), None));
        assert!(!sent.answered_by(1, Some(9), Some(6)));
        assert!(!sent.answered_by(2, Some(5), Some(6)));
    }
}
