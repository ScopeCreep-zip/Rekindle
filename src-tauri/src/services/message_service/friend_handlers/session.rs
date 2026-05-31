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

/// Process friend accept: establish *responder-side* Signal session.
///
/// The acceptor was the initiator (they called `establish_session`), so we are
/// the responder. We use the ephemeral key they sent us to derive a matching
/// shared secret via `respond_to_session()`.
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
) {
    let peer_label = state_helpers::friend_display_name(state, sender_hex)
        .unwrap_or_else(|| format!("{}…", &sender_hex[..16.min(sender_hex.len())]));

    if ephemeral_key.is_empty() {
        // W16.10d — was silent warn. Per Signal/SimpleX consensus on
        // never-silent crypto failures, surface this as a typed event
        // the user actually sees. Most likely cause: the accepter is
        // running a pre-W16 build that doesn't include session-init
        // material in FriendAccept; without ephemeral_key we cannot
        // establish a Signal session and every subsequent encrypted
        // message from that peer will fail AEAD on our side.
        tracing::error!(
            from = %sender_hex,
            "FriendAccept missing ephemeral key — no Signal session (peer running incompatible build?)"
        );
        crate::event_dispatch::emit_live(
            app_handle,
            "notification-event",
            &crate::channels::NotificationEvent::SystemAlert {
                title: "Couldn't establish secure session".into(),
                body: format!(
                    "Friend-accept from {peer_label} arrived without session-init data. \
                     Their build may be incompatible. Open their friend menu and click \
                     'Reset Secure Session' to retry — verify their safety number first."
                ),
            },
        );
        return;
    }

    // Extract their identity key from the PreKeyBundle
    let their_identity_key = match serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(
        prekey_bundle_bytes,
    ) {
        Ok(bundle) => bundle.identity_key,
        Err(e) => {
            // W16.10d — surface to user (was silent warn). Cause:
            // PreKeyBundle bytes garbled in transit or wrong format.
            tracing::error!(from = %sender_hex, error = %e,
                "failed to parse PreKeyBundle from FriendAccept");
            crate::event_dispatch::emit_live(
                app_handle,
                "notification-event",
                &crate::channels::NotificationEvent::SystemAlert {
                    title: "Couldn't establish secure session".into(),
                    body: format!(
                        "Friend-accept from {peer_label} carried an unparseable prekey bundle. \
                         Use 'Reset Secure Session' from their friend menu to retry. \
                         Verify their safety number out-of-band first."
                    ),
                },
            );
            return;
        }
    };

    let signal = state.signal_manager.read();
    if let Some(handle) = signal.as_ref() {
        // W16.10e (fix C) — guard against re-running responder X3DH on a
        // session that's already up. Our `rekindle-crypto` Signal port
        // overwrites session storage on every `respond_to_session` call
        // AND consumes a fresh one-time prekey
        // (`session.rs:242: self.prekeys.remove_prekey(otpk_id)?`). If
        // the peer's FriendAccept retries (their FriendRequestReceived
        // ACK was lost; sync_service re-fires), running this twice
        // wipes the working session AND the second call fails with
        // "one-time prekey not found" because the otpk was consumed.
        //
        // Pattern matches libsignal's `SessionBuilder.java:116`
        // short-circuit (`hasSessionState(version, baseKey)` →
        // `return Optional.absent()`), adapted to our simpler primitive:
        // skip if we already have a session AND the peer's identity_key
        // matches the trusted record. The `delete_session` call (was
        // unconditional) is dropped — it's the symptom, not the cure;
        // with the guard in place there's nothing stale to delete.
        let already_established = handle.manager.has_session(sender_hex).unwrap_or(false)
            && handle
                .manager
                .is_trusted_identity(sender_hex, &their_identity_key)
                .unwrap_or(false);

        if already_established {
            tracing::info!(from = %sender_hex,
                "session already established for peer — skipping respond_to_session \
                 (W16.10e idempotency; preserves working session + one-time prekey)");
            return;
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
            }
            Err(e) => {
                // W16.10d — was silent warn. This is the most common
                // path to "AEAD decrypt failures on every message from
                // peer" — Alice never installed her responder session
                // for Bob, so Bob's encrypted typing/presence/DM all
                // fail AEAD. Surface explicitly so the user can act.
                tracing::error!(from = %sender_hex, error = %e,
                    "failed to establish responder Signal session — encrypted messages from peer will fail AEAD");
                crate::event_dispatch::emit_live(
                    app_handle,
                    "notification-event",
                    &crate::channels::NotificationEvent::SystemAlert {
                        title: "Couldn't establish secure session".into(),
                        body: format!(
                            "Failed to establish encrypted session with {peer_label}: {e}. \
                             Encrypted messages from them will fail until you re-handshake. \
                             Click 'Reset Secure Session' from their friend menu after verifying \
                             their safety number out-of-band."
                        ),
                    },
                );
            }
        }
    } else {
        // Signal manager not yet initialized — log but don't surface
        // (this only happens if FriendAccept arrives before login
        // completes; rare edge case).
        tracing::warn!(from = %sender_hex,
            "FriendAccept arrived but Signal manager not yet initialized; session not established");
    }
}
