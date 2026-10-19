//! Opening an inbound 1:1 envelope before anything acts on it: recipient,
//! signature and freshness, the replay guard, the block list, the payload's
//! sealing and Signal decrypt, and the friend gate.

use rekindle_codec::message::envelope::{MessagePayload, Sealing};
use rekindle_protocol::messaging::receiver::{
    parse_sealed_payload, process_incoming, received_sealing,
};
use rekindle_protocol::messaging::replay::Sighting;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::Prepared;
use crate::operations::dm;
use crate::operations::friend::{is_blocked, state, FriendHost};

/// Open `envelope_bytes`; `None` (logged) for any reason it is dropped.
pub(super) async fn prepare(host: &dyn FriendHost, envelope_bytes: &[u8]) -> Option<Prepared> {
    let my_key = match state::owner(host).and_then(|k| dm::recipient_key(&k)) {
        Ok(key) => key,
        Err(e) => {
            tracing::debug!(error = %e, "dropping envelope: no local identity");
            return None;
        }
    };
    let now = host.clock().now_ms();
    let envelope = match process_incoming(envelope_bytes, &my_key, now) {
        Ok(envelope) => envelope,
        Err(e) => {
            tracing::warn!(error = %e, "dropping incoming envelope");
            return None;
        }
    };
    let sender_hex = hex::encode(&envelope.sender_key);
    let sighting = host.stores().replay.lock().observe(
        &envelope.sender_key,
        &envelope.nonce,
        envelope.timestamp,
        now,
    );
    match sighting {
        Sighting::First => {}
        Sighting::Duplicate => {
            tracing::debug!(from = %sender_hex, "dropping duplicate envelope");
            return None;
        }
        Sighting::Full => {
            tracing::warn!(from = %sender_hex, "replay guard full — dropping envelope");
            return None;
        }
    }
    tracing::debug!(from = %sender_hex, payload_len = envelope.payload.len(), "processing verified envelope");

    match is_blocked(host, &sender_hex).await {
        Ok(false) => {}
        Ok(true) => {
            tracing::debug!(from = %sender_hex, "dropping message from blocked user");
            return None;
        }
        Err(e) => {
            // Fail closed: a sender whose block state is unknown is not let in.
            tracing::warn!(from = %sender_hex, error = %e, "block list unreadable; dropping message");
            return None;
        }
    }

    let received = received_sealing(&envelope.payload);
    let opened = match received {
        Sealing::Plain => envelope.payload.clone(),
        Sealing::Session => decrypt(host, &sender_hex, &envelope.payload).await?,
    };
    let payload = match parse_sealed_payload(&opened, received) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::warn!(error = %e, from = %sender_hex, "dropping message payload");
            return None;
        }
    };

    // The gate reads the stored row (the Host's friend authority), so a
    // message dispatched before the projection loads is judged on it.
    let status = friendship_authoritative(host, &sender_hex).await;
    if !sender_admitted(&payload, status) {
        tracing::info!(from = %sender_hex, ?status, kind = payload_kind(&payload),
            "dropping message: sender not admitted for this payload");
        return None;
    }
    Some(Prepared {
        sender_hex,
        payload,
        timestamp: envelope.timestamp.try_into().unwrap_or(i64::MAX),
    })
}

/// The sender's friendship as the identity's database holds it; `None`
/// (fail closed) when it cannot be read.
async fn friendship_authoritative(host: &dyn FriendHost, sender: &str) -> Option<FriendshipState> {
    let store = host.friend_store().ok()?;
    let owner = state::owner(host).ok()?;
    match store.status_of(&owner, sender).await {
        Ok(status) => status,
        Err(e) => {
            tracing::warn!(error = %e, from = %sender, "friend status unreadable; failing closed");
            None
        }
    }
}

/// Whether a sender with friendship `status` may deliver `payload`.
///
/// - The friend-handshake and invitation payloads come from anyone not
///   blocked (blocking is checked before this).
/// - `FriendAccept` comes from a sender we sent a request to
///   (`PendingOut`) or one already `Accepted`: the higher side of a
///   crossing request is `Accepted` with no session when the lower side's
///   accept arrives. Never from a stranger or a `Removing` row (plan
///   C7.25).
/// - Everything else needs `Accepted`.
pub(super) fn sender_admitted(payload: &MessagePayload, status: Option<FriendshipState>) -> bool {
    match payload {
        MessagePayload::FriendRequest { .. }
        | MessagePayload::FriendRequestReceived
        | MessagePayload::Unfriended
        | MessagePayload::UnfriendedAck
        | MessagePayload::FriendReject
        | MessagePayload::RelayEnvelope { .. }
        | MessagePayload::GroupDmInvite { .. }
        | MessagePayload::WakeNotify { .. } => true,
        MessagePayload::FriendAccept { .. } => matches!(
            status,
            Some(FriendshipState::PendingOut | FriendshipState::Accepted)
        ),
        _ => status == Some(FriendshipState::Accepted),
    }
}

/// The payload's variant name, for the drop log.
fn payload_kind(payload: &MessagePayload) -> &'static str {
    match payload {
        MessagePayload::FriendAccept { .. } => "FriendAccept",
        MessagePayload::DirectMessage { .. } => "DirectMessage",
        MessagePayload::SessionResetRequest { .. } => "SessionResetRequest",
        MessagePayload::SessionResetAccept { .. } => "SessionResetAccept",
        _ => "other",
    }
}

/// Signal-decrypt a session-sealed payload; `None` after telling the user
/// when it does not open.
async fn decrypt(host: &dyn FriendHost, sender: &str, sealed: &[u8]) -> Option<Vec<u8>> {
    let Ok(signal) = host.signal() else {
        tracing::warn!(from = %sender, "received session-sealed payload but no signal manager");
        return None;
    };
    match signal.decrypt(sender, sealed).await {
        Ok(plaintext) => Some(plaintext),
        Err(e) => {
            tracing::error!(error = %e, from = %sender, payload_len = sealed.len(),
                "encrypted message could not be decrypted (Signal AEAD failure)");
            let from_label = state::label(host, sender);
            host.publish(SubscriptionEvent::Notification(
                NotificationEvent::SystemAlert {
                    title: "Message Decrypt Failed".to_string(),
                    body: format!(
                        "A message from {from_label} could not be decrypted. The secure \
                     session is missing or corrupted on this side. Open their friend \
                     menu and click 'Reset Secure Session' — verify their safety \
                     number out-of-band before re-establishing."
                    ),
                },
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept() -> MessagePayload {
        MessagePayload::FriendAccept {
            prekey_bundle: Vec::new(),
            profile_dht_key: String::new(),
            route_blob: Vec::new(),
            mailbox_dht_key: String::new(),
            ephemeral_key: Vec::new(),
            signed_prekey_id: 1,
            one_time_prekey_id: None,
            ml_kem_ciphertext: Vec::new(),
            used_ot_pqpk_id: None,
        }
    }

    fn dm() -> MessagePayload {
        MessagePayload::DirectMessage {
            body: "hi".into(),
            reply_to: None,
        }
    }

    #[test]
    fn friend_accept_reaches_the_requester() {
        assert!(sender_admitted(
            &accept(),
            Some(FriendshipState::PendingOut)
        ));
        // The higher side of a crossing request is already Accepted.
        assert!(sender_admitted(&accept(), Some(FriendshipState::Accepted)));
    }

    #[test]
    fn friend_accept_from_a_stranger_or_a_removed_row_drops() {
        assert!(!sender_admitted(&accept(), None));
        assert!(!sender_admitted(&accept(), Some(FriendshipState::Removing)));
    }

    #[test]
    fn a_pending_sender_may_send_nothing_but_the_accept() {
        assert!(!sender_admitted(&dm(), Some(FriendshipState::PendingOut)));
        assert!(!sender_admitted(&dm(), None));
        assert!(sender_admitted(&dm(), Some(FriendshipState::Accepted)));
    }
}
