//! Phase 14 — group call signaling handlers (W12.9 + W13.13).
//!
//! Group calls reuse the voice/video transport with a single shared
//! `call_key`. The initiator generates the key once, then sends each
//! invitee a `GroupCallOffer` carrying that invitee's per-recipient
//! `wrapped_call_key` (sealed via X25519 + HKDF + AES-256-GCM in
//! [`crate::group::wrap_call_key`]). Receivers unwrap with their
//! identity X25519 secret.
//!
//! Pre-Phase-14 these lived in `src-tauri/services/group_calls.rs`.
//! Parameterized over [`CallSignalingDeps`] for the same reasons as
//! the 1:1 handlers — the crate stays free of `AppState`, `tauri::
//! AppHandle`, and `rekindle-crypto::Identity` plumbing.

use std::collections::HashSet;

use rekindle_protocol::messaging::envelope::MessagePayload;
use x25519_dalek::StaticSecret;

use crate::group::unwrap_call_key;
use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::event::CallSignalEvent;
use crate::state::CallKind;

/// Dispatcher for group-call MessagePayload variants. After W13.13,
/// ALL group call signaling travels via app_message (was app_call in
/// W12.9). Non-group payloads log + return (callers shouldn't dispatch
/// them here, but we no longer `unreachable!()` since this is a public
/// trait-using function and panicking on bad input is hostile).
pub async fn handle_group_call_payload<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    payload: MessagePayload,
) {
    match payload {
        MessagePayload::GroupCallOffer {
            call_id,
            offer_kind,
            initiator_pubkey,
            initiator_x25519_pub,
            participants,
            wrapped_call_key,
            expires_at_ms,
        } => {
            handle_incoming_group_invite(
                deps,
                sender_hex,
                &call_id,
                offer_kind,
                &initiator_pubkey,
                &initiator_x25519_pub,
                participants,
                &wrapped_call_key,
                expires_at_ms,
            );
        }
        MessagePayload::GroupCallAccept {
            call_id,
            acceptor_pubkey,
        } => {
            handle_group_accept_received(deps, sender_hex, &call_id, &acceptor_pubkey);
        }
        MessagePayload::GroupCallDecline { call_id, reason } => {
            handle_group_decline_received(deps, sender_hex, &call_id, reason);
        }
        MessagePayload::GroupCallParticipantJoined {
            call_id,
            participant_pubkey,
        } => {
            // Defend against a malicious peer announcing arbitrary joins:
            // only forward if the announced participant is in the call's
            // known invite list.
            let snapshot = deps.group_registry().snapshot(&call_id);
            let known = snapshot.is_some_and(|s| s.participants.contains(&participant_pubkey));
            if known {
                deps.emit_event(CallSignalEvent::GroupCallParticipantJoined {
                    call_id,
                    peer_public_key: participant_pubkey,
                });
            }
        }
        MessagePayload::GroupCallParticipantLeft {
            call_id,
            participant_pubkey,
            reason,
        } => {
            let snapshot = deps.group_registry().snapshot(&call_id);
            let known = snapshot.is_some_and(|s| s.participants.contains(&participant_pubkey));
            if known {
                deps.emit_event(CallSignalEvent::GroupCallParticipantLeft {
                    call_id,
                    peer_public_key: participant_pubkey,
                    reason,
                });
            }
        }
        other => {
            tracing::warn!(payload = ?other, "handle_group_call_payload: non-group payload ignored");
        }
    }
}

/// Receiver-side entry into a group call. Mirrors the 1:1
/// `handle_incoming_invite` shape but with the per-recipient X25519
/// unwrap.
#[allow(clippy::too_many_arguments, reason = "GroupCallOffer envelope has 8 distinct wire fields; bundling adds indirection without arg-count win")]
pub fn handle_incoming_group_invite<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
    offer_kind: u8,
    initiator_pubkey: &str,
    initiator_x25519_pub: &[u8],
    participants: Vec<String>,
    wrapped_call_key: &[u8],
    expires_at_ms: u64,
) {
    // `offer_kind` is the wire-format u8 (0=audio, 1=video). The
    // GroupCallState + emitted IncomingGroupCall event both carry the
    // raw u8; we don't need the typed CallKind here, just validate it
    // round-trips. Decoding failure (unknown variant) defaults to
    // Audio to match 1:1 behavior.
    let _ = CallKind::from_u8(offer_kind).unwrap_or(CallKind::Audio);
    let display_name = {
        let name = deps.friend_display_name(sender_hex);
        if name.is_empty() {
            if initiator_pubkey.len() > 16 {
                format!("{}…", &initiator_pubkey[..16])
            } else {
                initiator_pubkey.to_string()
            }
        } else {
            name
        }
    };

    if initiator_x25519_pub.len() != 32 {
        tracing::warn!(call_id = %call_id, "GroupCallOffer with bad x25519 length");
        return;
    }

    // Temp-mute: silently drop if peer is muted (mirrors 1:1).
    if deps.is_peer_temp_muted(sender_hex) {
        return;
    }

    // Identity material — convert Ed25519 secret → X25519. The deps
    // trait gives us raw secret bytes; we convert via rekindle-crypto
    // (already a dep). The X25519 secret + Ed25519 public key are
    // both needed for unwrap_call_key.
    let Ok(our_ed_pubkey) = deps.owner_key() else {
        return;
    };
    let Ok(secret_bytes) = deps.identity_secret() else {
        return;
    };
    let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
    let our_x25519_secret = identity.to_x25519_secret();

    let call_key = match unwrap_call_key(
        &our_x25519_secret,
        initiator_x25519_pub,
        call_id,
        &our_ed_pubkey,
        wrapped_call_key,
    ) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(call_id = %call_id, error = %e,
                "group call key unwrap failed (likely not a participant)");
            return;
        }
    };

    deps.group_registry().insert(GroupCallState {
        call_id: call_id.to_string(),
        initiator_pubkey: initiator_pubkey.to_string(),
        kind: offer_kind,
        participants: participants.clone(),
        accepted: HashSet::new(),
        // Reconstruct StaticSecret from raw bytes (to_x25519_secret
        // consumed the identity material; we need to keep ours).
        our_x25519_secret: Some(reconstruct_x25519_secret(&our_x25519_secret)),
        call_key: Some(call_key),
        status: GroupCallStatus::Incoming,
    });

    deps.emit_event(CallSignalEvent::IncomingGroupCall {
        call_id: call_id.to_string(),
        initiator_public_key: sender_hex.to_string(),
        initiator_display_name: display_name,
        participants,
        kind: offer_kind,
        expires_at_ms,
    });
    deps.surface_window_for_call(call_id);
}

/// Initiator side: a participant accepted. Updates the GroupCallState
/// (Outgoing → Active on first accept), emits GroupCallConnected on the
/// first transition + always emits ParticipantJoined so the grid
/// updates.
pub fn handle_group_accept_received<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
    acceptor_pubkey: &str,
) {
    let _ = sender_hex; // sender == acceptor in normal flow

    // Validate the acceptor is actually in our participant list.
    let registry = deps.group_registry();
    let Some(snapshot) = registry.snapshot(call_id) else {
        return;
    };
    if !snapshot.participants.contains(&acceptor_pubkey.to_string()) {
        tracing::warn!(call = %call_id, acceptor = %acceptor_pubkey,
            "GroupCallAccept from non-invitee; ignoring");
        return;
    }

    let became_active = registry.add_accept(call_id, acceptor_pubkey);
    if became_active {
        // First accept — promote Outgoing → Active so downstream code
        // (in-call panel logic, audio teardown gates) sees the right
        // state. Old src-tauri handler did this inline; the crate
        // splits it into add_accept (returns "was empty") + explicit
        // set_status to keep the registry trait's mutation surface
        // narrow.
        registry.set_status(call_id, GroupCallStatus::Active);
        deps.emit_event(CallSignalEvent::GroupCallConnected {
            call_id: call_id.to_string(),
        });
    }
    deps.emit_event(CallSignalEvent::GroupCallParticipantJoined {
        call_id: call_id.to_string(),
        peer_public_key: acceptor_pubkey.to_string(),
    });
}

/// Initiator side: a participant declined.
pub fn handle_group_decline_received<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
    reason: String,
) {
    if !deps.group_registry().contains(call_id) {
        return;
    }
    deps.emit_event(CallSignalEvent::GroupCallParticipantLeft {
        call_id: call_id.to_string(),
        peer_public_key: sender_hex.to_string(),
        reason,
    });
}

/// Helper — clone an X25519 StaticSecret from another's bytes. The
/// `to_x25519_secret()` call in handle_incoming_group_invite consumes
/// the identity but returns a `StaticSecret`; we need a second copy to
/// store on `GroupCallState` (StaticSecret IS Clone but we go through
/// bytes to keep the semantic explicit + survive future API changes).
fn reconstruct_x25519_secret(src: &StaticSecret) -> StaticSecret {
    StaticSecret::from(src.to_bytes())
}
