//! Phase 14.n — caller-side 1:1 call entry points.
//!
//! `start_dm_call` is the W13.3 fire-and-forget caller flow ported
//! from `src-tauri/commands/calls.rs::start_dm_call` (107 LoC) so
//! the Tauri command can collapse to ~10 LoC of adapter construction.

use std::collections::HashSet;

use crate::error::CallError;
use crate::group::{generate_call_key, wrap_call_key};
use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::event::CallSignalEvent;
use crate::state::{CallKind, CallState, CallStatus};
use crate::X25519StaticSecret;
use crate::{derive_call_key, fresh_keypair, short_pubkey_helper};
use rekindle_protocol::messaging::envelope::MessagePayload;

const RING_DURATION_MS: u64 = 30_000;

/// W13.3 — start a 1:1 DM call. Inserts `CallState=Outgoing`, schedules
/// the 30 s dialing timeout, fires `CallInvite` via app_message, emits
/// `CallStarted` + `ConversationFocusRequested` so frontends seed the
/// outgoing-call UI. Returns `call_id` on success.
pub async fn start_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    peer_public_key: &str,
    kind: CallKind,
) -> Result<String, CallError> {
    let call_id = deps.fresh_call_id();
    let (my_secret, my_pub) = fresh_keypair();
    let initiator_pubkey = deps.owner_key()?;
    let expires_at_ms = rekindle_utils::timestamp_ms() + RING_DURATION_MS;

    // State BEFORE send so a fast peer ack finds the entry.
    deps.registry().insert(CallState {
        call_id: call_id.clone(),
        peer_pubkey: peer_public_key.to_string(),
        kind,
        status: CallStatus::Outgoing,
        expires_at_ms,
        my_x25519_secret: Some(my_secret),
        peer_x25519_pub: None,
        call_key: None,
    });

    deps.spawn_dialing_call_timeout(
        call_id.clone(),
        peer_public_key.to_string(),
        kind,
        expires_at_ms,
    );

    let invite = MessagePayload::CallInvite {
        call_id: call_id.clone(),
        offer_kind: kind.as_u8(),
        initiator_pubkey,
        initiator_x25519_pub: my_pub.to_vec(),
        expires_at_ms,
    };
    if let Err(e) = deps.send_to_peer(peer_public_key, invite).await {
        deps.registry().remove(&call_id);
        return Err(CallError::Transport(format!("call invite send failed: {e}")));
    }

    let display_name = {
        let name = deps.friend_display_name(peer_public_key);
        if name.is_empty() {
            short_pubkey_helper(peer_public_key)
        } else {
            name
        }
    };

    deps.emit_event(CallSignalEvent::CallStarted {
        call_id: call_id.clone(),
        peer_public_key: peer_public_key.to_string(),
        peer_display_name: display_name.clone(),
        kind,
        expires_at_ms,
    });

    deps.emit_event(CallSignalEvent::ConversationFocusRequested {
        peer_public_key: peer_public_key.to_string(),
        peer_display_name: display_name,
        reason: "call-started".to_string(),
    });

    Ok(call_id)
}

/// W13.5 — receiver-side LOCAL accept. The user clicked "Accept"
/// on the incoming-call modal: derive call_key, start voice session,
/// fire `CallAccept` envelope, transition to Active, emit
/// `CallConnected` + `ConversationFocusRequested`. Distinct from
/// `handle_accept_received` (caller-side handler of the peer's
/// CallAccept reply).
pub async fn accept_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
) -> Result<(), CallError> {
    let registry = deps.registry();

    // Validate + capture state under one lock.
    let (peer_pubkey, peer_x25519) = {
        let Some(call) = registry.get(call_id) else {
            return Err(CallError::CallNotFound(call_id.to_string()));
        };
        if !matches!(call.status, CallStatus::Incoming) {
            return Err(CallError::InvalidState(
                "call is not in Incoming state".into(),
            ));
        }
        let peer_x25519 = call.peer_x25519_pub.ok_or_else(|| {
            CallError::MalformedPayload("invite missing peer x25519".into())
        })?;
        (call.peer_pubkey.clone(), peer_x25519)
    };

    let (my_secret, my_pub) = fresh_keypair();
    let call_key = derive_call_key(&my_secret, &peer_x25519, call_id)?;

    // Persist the new state on the registry entry: my_secret + call_key
    // + status=Connecting. Read kind back out for the voice session
    // start + the final CallConnected emit.
    let kind = if let Some(mut call) = registry.get(call_id) {
        call.my_x25519_secret = Some(my_secret);
        call.call_key = Some(call_key);
        call.status = CallStatus::Connecting;
        let kind = call.kind;
        registry.insert(call);
        kind
    } else {
        return Err(CallError::CallNotFound(call_id.to_string()));
    };

    // Start voice BEFORE sending the accept so we're ready for the
    // caller's first packets.
    if let Err(e) = deps
        .start_voice_session(call_id, &peer_pubkey, call_key, kind)
        .await
    {
        registry.remove(call_id);
        deps.shutdown_voice_session().await;
        // Tell the caller we couldn't accept.
        let _ = deps
            .send_to_peer(
                &peer_pubkey,
                MessagePayload::CallDecline {
                    call_id: call_id.to_string(),
                    reason: format!("voice session failed: {e}"),
                },
            )
            .await;
        return Err(CallError::Session(format!("voice session failed: {e}")));
    }

    // Voice up. Fire CallAccept (caller will derive matching key).
    let accept = MessagePayload::CallAccept {
        call_id: call_id.to_string(),
        acceptor_x25519_pub: my_pub.to_vec(),
    };
    if let Err(e) = deps.send_to_peer(&peer_pubkey, accept).await {
        // We're locally in-call but the peer never got the accept.
        // Don't unwind — audio may still reach them via any other
        // path; caller will time out their dialing if not.
        tracing::warn!(call = %call_id, error = %e,
            "CallAccept send failed; caller may time out");
    }

    // Local UI transitions to Active.
    if let Some(mut call) = registry.get(call_id) {
        call.status = CallStatus::Active;
        registry.insert(call);
    }

    deps.emit_event(CallSignalEvent::CallConnected {
        call_id: call_id.to_string(),
        peer_public_key: peer_pubkey.clone(),
        kind,
    });
    let display_name = {
        let name = deps.friend_display_name(&peer_pubkey);
        if name.is_empty() {
            short_pubkey_helper(&peer_pubkey)
        } else {
            name
        }
    };
    deps.emit_event(CallSignalEvent::ConversationFocusRequested {
        peer_public_key: peer_pubkey,
        peer_display_name: display_name,
        reason: "call-accepted".to_string(),
    });

    Ok(())
}

/// W13.7 — receiver-side LOCAL decline. Drops the CallState entry
/// + fires `CallDecline` envelope. Fire-and-forget send (the peer's
/// state will GC via the dialing timeout if our envelope is lost).
pub async fn decline_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let peer_pubkey = deps
        .registry()
        .remove(call_id)
        .map(|c| c.peer_pubkey.clone())
        .ok_or_else(|| CallError::CallNotFound(call_id.to_string()))?;
    let _ = deps
        .send_to_peer(
            &peer_pubkey,
            MessagePayload::CallDecline {
                call_id: call_id.to_string(),
                reason: reason.unwrap_or_default(),
            },
        )
        .await;
    Ok(())
}

/// Hangup (or cancel-while-dialing). Removes CallState, fires
/// CallEnd to peer, tears down voice if it was up, emits CallEnded.
/// Works in any state (Outgoing / Incoming / Connecting / Active).
pub async fn end_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let removed = deps
        .registry()
        .remove(call_id)
        .ok_or_else(|| CallError::CallNotFound(call_id.to_string()))?;
    let was_voice_up = matches!(
        removed.status,
        CallStatus::Active | CallStatus::Connecting
    );
    let reason_str = reason.unwrap_or_default();

    let peer_pubkey = removed.peer_pubkey.clone();
    drop(removed); // Release Drop+Zeroize on the removed CallState now.

    if let Err(e) = deps
        .send_to_peer(
            &peer_pubkey,
            MessagePayload::CallEnd {
                call_id: call_id.to_string(),
                reason: reason_str.clone(),
            },
        )
        .await
    {
        tracing::info!(call = %call_id, peer = %peer_pubkey, error = %e,
            "CallEnd send failed; their state will GC on their own timeout");
    }

    // W15.2 — tear down voice so mic/speaker/loops actually stop.
    // Skip if voice was never up (Outgoing/Incoming status only).
    if was_voice_up {
        deps.shutdown_voice_session().await;
    }

    deps.emit_event(CallSignalEvent::CallEnded {
        call_id: call_id.to_string(),
        peer_public_key: peer_pubkey,
        reason: reason_str,
        voice_was_up: was_voice_up,
    });

    Ok(())
}

// ── Group calls (W12.9 / W13.13) ─────────────────────────────────────

/// Start a group call. Generates a shared 32-byte `call_key`,
/// builds `GroupCallState=Outgoing`, fans out per-recipient
/// `GroupCallOffer` envelopes each carrying the call_key sealed via
/// X25519 + HKDF-SHA256 + AES-256-GCM (`rekindle_calls::group::wrap_call_key`).
/// Fire-and-forget sends; accepts/declines arrive asynchronously
/// via `handle_group_call_payload`.
pub async fn start_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    participant_pubkeys: Vec<String>,
    kind: CallKind,
) -> Result<String, CallError> {
    let call_id = deps.fresh_call_id();
    let (my_secret, my_pub) = fresh_keypair();
    let initiator_pubkey = deps.owner_key()?;
    let expires_at_ms = rekindle_utils::timestamp_ms() + RING_DURATION_MS;
    let call_key = generate_call_key();

    let mut all_participants = vec![initiator_pubkey.clone()];
    for pk in &participant_pubkeys {
        if !all_participants.contains(pk) {
            all_participants.push(pk.clone());
        }
    }

    // Capture secret bytes for the per-invitee wrap before moving
    // `my_secret` into the registry entry.
    let secret_bytes = my_secret.to_bytes();
    deps.group_registry().insert(GroupCallState {
        call_id: call_id.clone(),
        initiator_pubkey: initiator_pubkey.clone(),
        kind: kind.as_u8(),
        participants: all_participants.clone(),
        accepted: HashSet::new(),
        our_x25519_secret: Some(my_secret),
        call_key: Some(call_key),
        status: GroupCallStatus::Outgoing,
    });

    for invitee in &participant_pubkeys {
        let Ok(invitee_ed_bytes) = hex::decode(invitee) else {
            tracing::warn!(peer = %invitee, "invalid Ed25519 pubkey hex; skipping");
            continue;
        };
        let Ok(invitee_ed_arr) = <[u8; 32]>::try_from(invitee_ed_bytes.as_slice()) else {
            tracing::warn!(peer = %invitee, "Ed25519 pubkey not 32 bytes; skipping");
            continue;
        };
        let invitee_x25519 = match rekindle_crypto::Identity::peer_ed25519_to_x25519(&invitee_ed_arr) {
            Ok(p) => p.to_bytes().to_vec(),
            Err(e) => {
                tracing::warn!(peer = %invitee, error = %e,
                    "Ed25519→X25519 conversion failed");
                continue;
            }
        };
        let secret = X25519StaticSecret::from(secret_bytes);
        let wrapped = match wrap_call_key(&secret, &invitee_x25519, &call_id, invitee, &call_key) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, peer = %invitee, "failed to wrap group call_key");
                continue;
            }
        };
        let offer = MessagePayload::GroupCallOffer {
            call_id: call_id.clone(),
            offer_kind: kind.as_u8(),
            initiator_pubkey: initiator_pubkey.clone(),
            initiator_x25519_pub: my_pub.to_vec(),
            participants: all_participants.clone(),
            wrapped_call_key: wrapped,
            expires_at_ms,
        };
        if let Err(e) = deps.send_to_peer(invitee, offer).await {
            tracing::info!(peer = %invitee, error = %e, "GroupCallOffer send failed");
        }
    }

    Ok(call_id)
}

/// W13.13 — receiver-side LOCAL accept of a group call. Transitions
/// Incoming → Active, fires `GroupCallAccept` to the initiator,
/// emits `GroupCallConnected`. Voice session setup is W14
/// follow-up — left to existing 1:1 voice flow until group-voice
/// topology lands.
pub async fn accept_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
) -> Result<(), CallError> {
    let registry = deps.group_registry();
    let snapshot = registry
        .snapshot(call_id)
        .ok_or_else(|| CallError::CallNotFound(call_id.to_string()))?;
    if snapshot.status != GroupCallStatus::Incoming {
        return Err(CallError::InvalidState(
            "group call is not in Incoming state".into(),
        ));
    }
    registry.set_status(call_id, GroupCallStatus::Active);

    let our_ed_pubkey = deps.owner_key()?;
    let payload = MessagePayload::GroupCallAccept {
        call_id: call_id.to_string(),
        acceptor_pubkey: our_ed_pubkey,
    };
    if let Err(e) = deps.send_to_peer(&snapshot.initiator_pubkey, payload).await {
        tracing::warn!(call = %call_id, error = %e, "GroupCallAccept send failed");
    }

    deps.emit_event(CallSignalEvent::GroupCallConnected {
        call_id: call_id.to_string(),
    });
    Ok(())
}

/// W13.13 — receiver-side LOCAL decline of a group call. Drops the
/// GroupCallState entry + fires `GroupCallDecline` to the initiator.
pub async fn decline_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let removed = deps
        .group_registry()
        .remove(call_id)
        .ok_or_else(|| CallError::CallNotFound(call_id.to_string()))?;
    let initiator = removed.initiator_pubkey.clone();
    drop(removed);
    let _ = deps
        .send_to_peer(
            &initiator,
            MessagePayload::GroupCallDecline {
                call_id: call_id.to_string(),
                reason: reason.unwrap_or_default(),
            },
        )
        .await;
    Ok(())
}

/// Leave / end a group call locally. Drops the GroupCallState
/// entry and emits `GroupCallEnded` so the local UI dismisses the
/// in-call panel. Other participants learn through whatever path
/// our exit registers (gossip ParticipantLeft on next presence
/// poll, dialing-timeout on the initiator's side, etc.). No
/// explicit GroupCallEnd envelope today — matches the pre-Phase-14
/// behavior.
pub fn end_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let removed = deps
        .group_registry()
        .remove(call_id)
        .ok_or_else(|| CallError::CallNotFound(call_id.to_string()))?;
    drop(removed);
    deps.emit_event(CallSignalEvent::GroupCallEnded {
        call_id: call_id.to_string(),
        reason: reason.unwrap_or_default(),
    });
    Ok(())
}
