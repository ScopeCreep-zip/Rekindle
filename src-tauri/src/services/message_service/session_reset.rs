//! Phase 23.a — session reset payload handlers + safety-number helper.
//!
//! Pre-port these lived inline in `services/message_service.rs`
//! (~210 LoC). Lifted into a sibling module so the parent file
//! stays focused on inbound dispatch + the public send API.
//!
//! P3.3 — out-of-band safety-number flow:
//! - `handle_session_reset_request` stashes the requester's
//!   PreKeyBundle in `pending_session_resets` and emits a
//!   `SessionResetRequested` notification with the safety number.
//! - `handle_session_reset_accept` installs the fresh responder-side
//!   session matching the peer's new initiator-side session.
//! - `handle_session_reset_payload` dispatches between the three
//!   variants.
//! - `compute_safety_number` derives the short out-of-band
//!   verification code shared by both sides.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::state::AppState;
use crate::state_helpers;

pub(super) fn handle_session_reset_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    sender_hex: &str,
    payload: MessagePayload,
) {
    match payload {
        MessagePayload::SessionResetRequest { our_prekey_bundle } => {
            handle_session_reset_request(app_handle, state, sender_hex, &our_prekey_bundle);
        }
        MessagePayload::SessionResetAccept {
            ephemeral_key,
            signed_prekey_id,
            one_time_prekey_id,
            our_identity_key,
            ml_kem_ciphertext,
            used_ot_pqpk_id,
        } => {
            handle_session_reset_accept(
                app_handle,
                state,
                sender_hex,
                &ephemeral_key,
                signed_prekey_id,
                one_time_prekey_id,
                &our_identity_key,
                &ml_kem_ciphertext,
                used_ot_pqpk_id,
            );
        }
        MessagePayload::SessionResetDecline { reason } => {
            let display = state_helpers::friend_display_name(state, sender_hex)
                .unwrap_or_else(|| format!("{}...", &sender_hex[..8.min(sender_hex.len())]));
            let body = if reason.is_empty() {
                format!("{display} declined your secure session reset request.")
            } else {
                format!("{display} declined your secure session reset request: {reason}")
            };
            crate::event_dispatch::emit_live(
                app_handle,
                "notification-event",
                &crate::channels::NotificationEvent::SystemAlert {
                    title: "Session Reset Declined".to_string(),
                    body,
                },
            );
        }
        _ => unreachable!("handle_session_reset_payload called with non-session payload"),
    }
}

/// P3.3 — receive a SessionResetRequest from a peer. Stash the
/// requester's PreKeyBundle in `pending_session_resets` (in-memory
/// only, never persisted before user consent) and emit a
/// notification so the user can confirm via `accept_session_reset`
/// after verifying the safety number out-of-band. Drops requests
/// from non-friends per the strict allowlist.
fn handle_session_reset_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    sender_hex: &str,
    prekey_bundle: &[u8],
) {
    if !state_helpers::is_friend(state, sender_hex) {
        tracing::debug!(
            from = %sender_hex,
            "dropping SessionResetRequest from non-friend",
        );
        return;
    }
    if prekey_bundle.is_empty() {
        tracing::warn!(
            from = %sender_hex,
            "SessionResetRequest with empty prekey bundle — dropping",
        );
        return;
    }
    // Stash the bundle for the user's accept_session_reset command
    // to consume. INSERT-OR-REPLACE: if the peer sent multiple reset
    // requests (e.g., a retry after their UI didn't see our reply),
    // the most recent bundle wins.
    state
        .pending_session_resets
        .lock()
        .insert(sender_hex.to_string(), prekey_bundle.to_vec());

    let display_name = state_helpers::friend_display_name(state, sender_hex)
        .unwrap_or_else(|| format!("{}...", &sender_hex[..8.min(sender_hex.len())]));
    let safety_number = compute_safety_number(state, sender_hex, prekey_bundle)
        .unwrap_or_else(|| "<unavailable>".to_string());

    crate::event_dispatch::emit_live(
        app_handle,
        "notification-event",
        &crate::channels::NotificationEvent::SessionResetRequested {
            peer_public_key: sender_hex.to_string(),
            peer_display_name: display_name,
            safety_number,
        },
    );
}

/// P3.3 — receive a SessionResetAccept from the peer we previously
/// sent a SessionResetRequest to. Installs a fresh responder-side
/// session matching the peer's new initiator-side session.
/// `delete_session` is idempotent on a missing peer, so repeated
/// accepts (e.g., due to a retry) just overwrite cleanly.
#[allow(
    clippy::too_many_arguments,
    reason = "matches MessagePayload::SessionResetAccept wire shape; collapsing into a struct would add boilerplate for a single call site"
)]
fn handle_session_reset_accept(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    sender_hex: &str,
    ephemeral_key: &[u8],
    signed_prekey_id: u32,
    one_time_prekey_id: Option<u32>,
    their_identity_key: &[u8],
    ml_kem_ciphertext: &[u8],
    used_ot_pqpk_id: Option<u32>,
) {
    if !state_helpers::is_friend(state, sender_hex) {
        tracing::debug!(
            from = %sender_hex,
            "dropping SessionResetAccept from non-friend",
        );
        return;
    }
    if ephemeral_key.is_empty() || their_identity_key.is_empty() {
        tracing::warn!(
            from = %sender_hex,
            "SessionResetAccept missing ephemeral or identity key",
        );
        return;
    }
    let signal = state.signal_manager.read();
    let Some(handle) = signal.as_ref() else {
        tracing::warn!(from = %sender_hex, "SessionResetAccept arrived but signal manager not initialized");
        return;
    };
    // Idempotent — clear any stale session before installing the fresh one.
    let _ = handle.manager.delete_session(sender_hex);
    match handle.manager.respond_to_session(
        sender_hex,
        their_identity_key,
        ephemeral_key,
        signed_prekey_id,
        one_time_prekey_id,
        ml_kem_ciphertext,
        used_ot_pqpk_id,
    ) {
        Ok(()) => {
            tracing::info!(
                from = %sender_hex,
                "established responder Signal session from SessionResetAccept",
            );
            let display = state_helpers::friend_display_name(state, sender_hex)
                .unwrap_or_else(|| format!("{}...", &sender_hex[..8.min(sender_hex.len())]));
            crate::event_dispatch::emit_live(
                app_handle,
                "notification-event",
                &crate::channels::NotificationEvent::SystemAlert {
                    title: "Secure session re-established".to_string(),
                    body: format!(
                        "New Signal session with {display} is active. Verify their safety number out-of-band before resuming sensitive conversations."
                    ),
                },
            );
        }
        Err(error) => {
            tracing::warn!(
                from = %sender_hex,
                %error,
                "failed to respond_to_session from SessionResetAccept",
            );
        }
    }
}

/// P3.3 — short safety number for out-of-band verification.
///
/// `BLAKE3(sort([our_identity_key, peer_identity_key]) ||
/// "rekindle-safety-v1")` → first 8 hex chars (32 bits = roughly 6
/// chars-worth of entropy by brute-force; small enough to read
/// aloud, large enough to detect substitution attacks at the cost
/// a casual user would tolerate).
///
/// Computed from a PreKeyBundle's identity_key field. Both sides
/// produce the same value because `sort` is order-independent.
fn compute_safety_number(
    state: &Arc<AppState>,
    _peer_hex: &str,
    peer_prekey_bundle: &[u8],
) -> Option<String> {
    let bundle: rekindle_crypto::signal::PreKeyBundle =
        serde_json::from_slice(peer_prekey_bundle).ok()?;
    let our_secret_bytes = (*state.identity_secret.lock())?;
    let our_identity = rekindle_crypto::Identity::from_secret_bytes(&our_secret_bytes);
    // Phase 3b — safety number derives from Ed25519 verifying keys
    // so both sides see the same input (the PreKeyBundle now
    // publishes Ed25519 identity bytes, not X25519).
    let our_pub_bytes = our_identity.public_key_bytes();
    let mut keys = [our_pub_bytes.as_slice(), bundle.identity_key.as_slice()];
    keys.sort();
    let mut hasher = blake3::Hasher::new();
    hasher.update(keys[0]);
    hasher.update(keys[1]);
    hasher.update(b"rekindle-safety-v1");
    let hash = hasher.finalize();
    Some(hex::encode(&hash.as_bytes()[..4]))
}
