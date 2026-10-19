//! The responder-side Signal session a `FriendAccept`'s init installs.

use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::IncomingFriendAccept;
use crate::operations::friend::{state, FriendHost};

/// What answering a `FriendAccept`'s session init came to.
pub(super) enum AcceptOutcome {
    /// We installed the responder session.
    Established,
    /// Not ours to answer: a retried accept for a session we already
    /// have. The friendship is unaffected.
    AlreadyEstablished,
    /// No session: the reason, for the friend row and the event.
    Failed(String),
}

/// Establish the responder-side session from an accept: the acceptor
/// initiated (`establish_session`), so we derive the matching secret from
/// the ephemeral key they sent.
///
/// A failure is first contact going wrong, not a broken trusted session,
/// so it never points the user at "Reset Secure Session" (the
/// trust-changing flow): the caller keeps the friendship pending and
/// publishes `FriendEvent::HandshakeFailed` (plan C7.25). Only an
/// identity-key mismatch is a trust event, and it gets its own notice.
pub(super) fn answer_accept(host: &dyn FriendHost, a: &IncomingFriendAccept<'_>) -> AcceptOutcome {
    let sender = a.sender_hex;
    if a.ephemeral_key.is_empty() {
        tracing::error!(from = %sender, "FriendAccept missing ephemeral key — no Signal session");
        return AcceptOutcome::Failed("their accept carried no session data".into());
    }
    let their_identity_key = match rekindle_crypto::signal::PreKeyBundle::from_wire(a.prekey_bundle)
    {
        Ok(bundle) => bundle.identity_key,
        Err(e) => {
            tracing::error!(from = %sender, error = %e, "failed to parse PreKeyBundle from FriendAccept");
            return AcceptOutcome::Failed("their accept carried an unreadable key bundle".into());
        }
    };
    let friendship = state::friendship(host, sender);
    let Ok(signal) = host.signal() else {
        tracing::warn!(from = %sender, "FriendAccept arrived with no Signal manager; session not established");
        return AcceptOutcome::Failed("not signed in when their accept arrived".into());
    };
    // We answer the init only while it is ours to answer: we sent the
    // request (`PendingOut`), or we are the higher side of a crossing
    // request (`Accepted`, no session yet). `Accepted` with a session is a
    // retried accept; answering it again would replace the working session
    // and fail on the one-time keys it already used.
    let respond = match friendship {
        Some(FriendshipState::PendingOut) => Ok(true),
        Some(FriendshipState::Accepted) => signal.has_session(sender).map(|has| !has),
        _ => Ok(false),
    };
    match respond {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(from = %sender, state = ?friendship,
                "FriendAccept not ours to answer (retry or unknown peer) — ignoring");
            return AcceptOutcome::AlreadyEstablished;
        }
        Err(e) => {
            tracing::error!(from = %sender, error = %e, "FriendAccept: session lookup failed");
            return AcceptOutcome::Failed(format!("could not read the session store: {e}"));
        }
    }
    match signal.respond_to_session(
        sender,
        &their_identity_key,
        a.ephemeral_key,
        a.signed_prekey_id,
        a.one_time_prekey_id,
        a.ml_kem_ciphertext,
        a.used_ot_pqpk_id,
    ) {
        Ok(()) => {
            tracing::info!(from = %sender, "established responder Signal session from FriendAccept");
            AcceptOutcome::Established
        }
        Err(rekindle_crypto::error::CryptoError::UntrustedIdentity { .. }) => {
            tracing::error!(from = %sender,
                "FriendAccept under an identity key other than the trusted one — refused");
            let peer_label = state::label(host, sender);
            host.publish(SubscriptionEvent::Notification(
                NotificationEvent::SystemAlert {
                    title: "Identity key changed".into(),
                    body: format!(
                        "A friend-accept for {peer_label} came under a different identity key \
                     than the one you already trust. Nothing was accepted. Compare safety \
                     numbers with them out of band before going further."
                    ),
                },
            ));
            AcceptOutcome::Failed("their identity key does not match the trusted one".into())
        }
        Err(e) => {
            tracing::error!(from = %sender, error = %e,
                "failed to establish responder Signal session from FriendAccept");
            AcceptOutcome::Failed(format!("the secure-session handshake failed: {e}"))
        }
    }
}
