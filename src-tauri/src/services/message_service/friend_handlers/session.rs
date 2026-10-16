//! Friend-handshake primitives: the `FriendRequest` log (no Signal
//! session installed yet) and the responder-side `FriendAccept`
//! installer that runs `respond_to_session()` on the Signal manager.

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

pub(super) fn handle_friend_request(sender_hex: &str, prekey_bundle_bytes: &[u8]) {
    if prekey_bundle_bytes.is_empty() {
        tracing::warn!(from = %sender_hex, "friend request has empty prekey bundle");
    } else {
        tracing::info!(
            from = %sender_hex,
            prekey_len = prekey_bundle_bytes.len(),
            "received friend request — prekey bundle stored for later session establishment"
        );
    }
}

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

/// Process friend accept: establish *responder-side* Signal session.
///
/// The acceptor was the initiator (they called `establish_session`), so we are
/// the responder. We use the ephemeral key they sent us to derive a matching
/// shared secret via `respond_to_session()`.
///
/// A failure is first contact going wrong, not a broken trusted session,
/// so it never points the user at "Reset Secure Session" (the
/// trust-changing flow): the caller keeps the friendship pending and
/// emits `FriendEvent::HandshakeFailed` (plan C7.25). Retrying a failed
/// first contact is E2.4's signed `PreKeyRejected`; a same-identity
/// FriendRequest would not do it, since the accepter treats one as a
/// network duplicate. Only an identity-key mismatch is a trust event, and
/// it gets its own notice.
pub(super) fn handle_friend_accept(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    sender_hex: &str,
    prekey_bundle_bytes: &[u8],
    ephemeral_key: &[u8],
    signed_prekey_id: u32,
    one_time_prekey_id: Option<u32>,
    ml_kem_ciphertext: &[u8],
    used_ot_pqpk_id: Option<u32>,
) -> AcceptOutcome {
    if ephemeral_key.is_empty() {
        tracing::error!(from = %sender_hex, "FriendAccept missing ephemeral key — no Signal session");
        return AcceptOutcome::Failed("their accept carried no session data".into());
    }

    let their_identity_key =
        match rekindle_crypto::signal::PreKeyBundle::from_wire(prekey_bundle_bytes) {
            Ok(bundle) => bundle.identity_key,
            Err(e) => {
                tracing::error!(from = %sender_hex, error = %e,
                "failed to parse PreKeyBundle from FriendAccept");
                return AcceptOutcome::Failed(
                    "their accept carried an unreadable key bundle".into(),
                );
            }
        };

    let friendship_state =
        state_helpers::friend_field(state, sender_hex, |f| Some(f.friendship_state));
    let signal = state.signal_manager.read();
    let Some(handle) = signal.as_ref() else {
        tracing::warn!(from = %sender_hex,
            "FriendAccept arrived but Signal manager not yet initialized; session not established");
        return AcceptOutcome::Failed("not signed in when their accept arrived".into());
    };
    // We answer the init only while it is ours to answer: we sent the
    // request (`PendingOut`), or we are the higher side of a crossing
    // request (`Accepted`, no session yet). `Accepted` with a session
    // is a retried `FriendAccept`; answering it again would replace the
    // working session and fail on the one-time keys it already used.
    let respond = match friendship_state {
        Some(crate::state::FriendshipState::PendingOut) => Ok(true),
        Some(crate::state::FriendshipState::Accepted) => {
            handle.manager.has_session(sender_hex).map(|has| !has)
        }
        _ => Ok(false),
    };
    match respond {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(from = %sender_hex, state = ?friendship_state,
                "FriendAccept not ours to answer (retry or unknown peer) — ignoring");
            return AcceptOutcome::AlreadyEstablished;
        }
        Err(e) => {
            tracing::error!(from = %sender_hex, error = %e, "FriendAccept: session lookup failed");
            return AcceptOutcome::Failed(format!("could not read the session store: {e}"));
        }
    }

    match handle.manager.respond_to_session(
        sender_hex,
        &their_identity_key,
        ephemeral_key,
        signed_prekey_id,
        one_time_prekey_id,
        ml_kem_ciphertext,
        used_ot_pqpk_id,
    ) {
        Ok(()) => {
            tracing::info!(from = %sender_hex, "established responder Signal session from FriendAccept");
            AcceptOutcome::Established
        }
        Err(rekindle_crypto::error::CryptoError::UntrustedIdentity { .. }) => {
            tracing::error!(from = %sender_hex,
                "FriendAccept under an identity key other than the trusted one — refused");
            let peer_label = state_helpers::friend_display_name(state, sender_hex)
                .unwrap_or_else(|| format!("{}…", rekindle_utils::text::prefix(sender_hex, 16)));
            crate::event_dispatch::emit_notification(
                app_handle,
                rekindle_types::subscription_events::NotificationEvent::SystemAlert {
                    title: "Identity key changed".into(),
                    body: format!(
                        "A friend-accept for {peer_label} came under a different identity key \
                         than the one you already trust. Nothing was accepted. Compare safety \
                         numbers with them out of band before going further."
                    ),
                },
            );
            AcceptOutcome::Failed("their identity key does not match the trusted one".into())
        }
        Err(e) => {
            tracing::error!(from = %sender_hex, error = %e,
                "failed to establish responder Signal session from FriendAccept");
            AcceptOutcome::Failed(format!("the secure-session handshake failed: {e}"))
        }
    }
}
