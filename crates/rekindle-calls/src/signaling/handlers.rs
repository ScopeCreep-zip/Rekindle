//! Phase 14 — 1:1 call signaling handlers.
//!
//! Receive arms for `CallInvite`/`CallAccept`/`CallDecline`/`CallRinging`
//! (architecture §10.10, Wave 13). Parameterized over
//! [`CallSignalingDeps`] so this crate stays free of `AppState`,
//! `tauri::AppHandle`, and direct `veilid-core` references. The
//! src-tauri adapter (lands in 14.h) implements the deps trait against
//! `AppState` + `message_service::send_to_peer_raw` + `services::voice::*`.
//!
//! Pre-Phase-14 these lived in `src-tauri/services/calls/mod.rs`.

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::event::CallSignalEvent;
use crate::state::{CallKind, CallState, CallStatus};

/// Truncate a hex pubkey for display when no friend display name is
/// known. One implementation, shared with the src-tauri adapter's
/// `emit_event` mappings — the crate root's doc comment already said it
/// was public for exactly that reason.
use crate::short_pubkey_helper as short_pubkey;

/// Decoded `CallInvite` wire fields for [`handle_incoming_invite`].
/// Borrows the envelope-owned strings/bytes so dispatch passes them
/// without copying.
pub struct IncomingInvite<'a> {
    /// Hex pubkey of the Veilid sender (envelope `from`).
    pub sender_hex: &'a str,
    /// Call identifier from the invite.
    pub call_id: &'a str,
    /// Wire offer kind: `0` = audio, `1` = video.
    pub offer_kind: u8,
    /// Initiator's Ed25519 identity pubkey (hex), used for display
    /// fallback.
    pub initiator_pubkey: &'a str,
    /// Initiator's X25519 ECDH public key (must be 32 bytes).
    pub initiator_x25519_pub: &'a [u8],
    /// Invite expiry, ms since epoch.
    pub expires_at_ms: u64,
    /// Phase 5 — codecs the initiator can DECODE (wire strings).
    /// Stored on the CallState so the accept-side video sender can
    /// intersect its encode set.
    pub video_decode_codecs: &'a [String],
}

/// Receive arm for `CallInvite` (W13.4). Receiver-side entry. Inserts
/// `CallState=Incoming`, emits IncomingCall events, fires a best-effort
/// `CallRinging` ack to the caller, schedules a 30 s timeout.
///
/// W12.12 temp-mute: silently auto-decline if the peer is on the mute
/// list. W13.15 glare: if we have an Outgoing call to the SAME peer
/// in flight, the side with the lower hex pubkey wins their outgoing;
/// the loser cancels theirs and processes this incoming.
///
/// Fire-and-forget — does not return a Result. All failures log and
/// drop.
pub async fn handle_incoming_invite<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    invite: IncomingInvite<'_>,
) {
    let IncomingInvite {
        sender_hex,
        call_id,
        offer_kind,
        initiator_pubkey,
        initiator_x25519_pub,
        expires_at_ms,
        video_decode_codecs,
    } = invite;
    let kind = CallKind::from_u8(offer_kind).unwrap_or(CallKind::Audio);
    let display_name = if deps.friend_display_name(sender_hex).is_empty() {
        short_pubkey(initiator_pubkey)
    } else {
        deps.friend_display_name(sender_hex)
    };

    if initiator_x25519_pub.len() != 32 {
        send_decline(deps, sender_hex, call_id, "invalid x25519 public key").await;
        return;
    }
    let mut peer_arr = [0u8; 32];
    peer_arr.copy_from_slice(initiator_x25519_pub);

    // W12.12 temp-mute: silently auto-decline without ringing.
    if deps.is_peer_temp_muted(sender_hex) {
        send_decline(deps, sender_hex, call_id, "user is unavailable").await;
        return;
    }

    // W13.15 glare resolution.
    let our_pubkey = deps.owner_key().unwrap_or_default();
    let registry = deps.registry();
    if let Some(outgoing) = registry.outgoing_to_peer(sender_hex) {
        let we_win = our_pubkey.as_str() < sender_hex;
        if we_win {
            send_decline(deps, sender_hex, call_id, "glare-resolved-other-loses").await;
            return;
        }
        // We lose — cancel our outgoing for this peer.
        cancel_outgoing_for_glare(deps, &outgoing.call_id, sender_hex).await;
    }

    // Insert CallState as Incoming. Receiver's local X25519 keypair is
    // generated at accept-time (Tauri command), not here.
    registry.insert(CallState {
        call_id: call_id.to_string(),
        peer_pubkey: sender_hex.to_string(),
        kind,
        status: CallStatus::Incoming,
        expires_at_ms,
        my_x25519_secret: None,
        peer_x25519_pub: Some(peer_arr),
        call_key: None,
        peer_video_decode_codecs: video_decode_codecs.to_vec(),
    });

    // Surface incoming-call UI (adapter emits ChatEvent::IncomingCall +
    // NotificationEvent::CallIncoming both as journaled events so a
    // page-reload mid-ring restores the panel).
    deps.emit_event(CallSignalEvent::IncomingCall {
        call_id: call_id.to_string(),
        from_public_key: sender_hex.to_string(),
        from_display_name: display_name,
        kind,
        expires_at_ms,
    });
    deps.surface_window_for_call(call_id);

    // Best-effort CallRinging ack so the caller's UI can flip
    // "Calling…" → "Ringing…". Failure logs but doesn't block.
    let ringing = MessagePayload::CallRinging {
        call_id: call_id.to_string(),
    };
    if let Err(e) = deps.send_to_peer(sender_hex, ringing).await {
        tracing::debug!(call = %call_id, error = %e, "CallRinging ack send failed");
    }

    // W13.2 — schedule local 30 s timeout. Delegated to the adapter
    // so it can do the full sleep → status-check → registry remove
    // → missed_calls persist → CallMissed emit sequence across the
    // tokio::spawn boundary (where `&dyn CallSignalingDeps` cannot
    // travel). See `CallSignalingDeps::spawn_incoming_call_timeout`
    // for the full contract.
    deps.spawn_incoming_call_timeout(
        call_id.to_string(),
        sender_hex.to_string(),
        kind,
        expires_at_ms,
    );
}

/// Receive arm for `CallAccept` (W13.6). Caller-side: peer accepted.
/// Derives the shared `call_key`, brings up voice, transitions to
/// Active. On any error, sends `CallEnd` so the peer's started voice
/// tears down too.
///
/// W14.1 — the voice receive channel is pre-staged BEFORE any await
/// points (inside `start_voice_session`'s adapter impl) so packets
/// arriving during session setup buffer rather than drop at dispatch.
pub async fn handle_accept_received<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
    acceptor_x25519_pub: &[u8],
    video_decode_codecs: &[String],
) {
    if acceptor_x25519_pub.len() != 32 {
        tracing::warn!(call = %call_id, "CallAccept with bad x25519 length");
        return;
    }

    // W14.1 — pre-stage the voice receive channel BEFORE any await
    // points so packets arriving during the rest of the accept handler
    // buffer in the channel rather than getting dropped at dispatch.
    // Adapter is responsible for the actual `voice_packet_tx/rx_staged`
    // mpsc setup on AppState.
    deps.pre_stage_voice_channel();

    let registry = deps.registry();
    let (my_secret, peer_pubkey, kind) = {
        let Some(call) = registry.get(call_id) else {
            tracing::debug!(call = %call_id, "CallAccept for unknown call; ignoring");
            return;
        };
        if !matches!(call.status, CallStatus::Outgoing) {
            tracing::debug!(call = %call_id, status = ?call.status,
                "CallAccept for call not in Outgoing state; ignoring");
            return;
        }
        if call.peer_pubkey != sender_hex {
            tracing::warn!(call = %call_id, expected = %call.peer_pubkey, actual = %sender_hex,
                "CallAccept from wrong peer; ignoring");
            return;
        }
        let Some(secret) = call.my_x25519_secret.clone() else {
            tracing::warn!(call = %call_id, "CallAccept but local x25519 secret missing");
            return;
        };
        (secret, call.peer_pubkey.clone(), call.kind)
    };

    // Derive call_key. On failure, drop the call + emit CallEnded.
    let call_key = match crate::derive_call_key(&my_secret, acceptor_x25519_pub, call_id) {
        Ok(k) => k,
        Err(e) => {
            registry.remove(call_id);
            deps.emit_event(CallSignalEvent::CallEnded {
                call_id: call_id.to_string(),
                peer_public_key: peer_pubkey,
                reason: format!("derive call_key: {e}"),
                voice_was_up: false,
            });
            return;
        }
    };

    // Update registry: store call_key + transition to Connecting.
    if let Some(mut call) = registry.get(call_id) {
        call.call_key = Some(call_key);
        call.status = CallStatus::Connecting;
        let mut peer_arr = [0u8; 32];
        peer_arr.copy_from_slice(acceptor_x25519_pub);
        call.peer_x25519_pub = Some(peer_arr);
        call.peer_video_decode_codecs = video_decode_codecs.to_vec();
        // my_x25519_secret was already cloned-out above; the registry
        // still has the original.
        registry.insert(call);
    }

    // Bring up voice. Adapter handles the W14.1 pre-stage internally.
    if let Err(e) = deps
        .start_voice_session(call_id, &peer_pubkey, call_key, kind)
        .await
    {
        registry.remove(call_id);
        deps.shutdown_voice_session().await;
        let hangup = MessagePayload::CallEnd {
            call_id: call_id.to_string(),
            reason: format!("voice session failed: {e}"),
        };
        let _ = deps.send_to_peer(&peer_pubkey, hangup).await;
        deps.emit_event(CallSignalEvent::CallEnded {
            call_id: call_id.to_string(),
            peer_public_key: peer_pubkey,
            reason: format!("voice session failed: {e}"),
            voice_was_up: false,
        });
        return;
    }

    // Promote to Active + emit CallConnected + focus chat.
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
            crate::short_pubkey_helper(&peer_pubkey)
        } else {
            name
        }
    };
    deps.emit_event(CallSignalEvent::ConversationFocusRequested {
        peer_public_key: peer_pubkey,
        peer_display_name: display_name,
        reason: "call-accepted".to_string(),
    });
}

/// Receive arm for `CallDecline` (W13.8). Caller-side. Idempotent;
/// silently no-ops on unknown call_id or wrong peer.
pub fn handle_decline_received<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
    reason: String,
) {
    let registry = deps.registry();
    let removed = registry
        .get(call_id)
        .filter(|c| c.peer_pubkey == sender_hex)
        .is_some()
        && registry.remove(call_id).is_some();
    if removed {
        deps.emit_event(CallSignalEvent::CallDeclined {
            call_id: call_id.to_string(),
            peer_public_key: sender_hex.to_string(),
            reason,
        });
    }
}

/// Receive arm for `CallRinging`. Caller-side alerting hint — receiver
/// has the invite and is ringing. Verifies the ringing is for a call
/// we actually own (defends against a forged ringing for someone else's
/// call_id).
pub fn handle_ringing_received<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
) {
    let registry = deps.registry();
    let known = registry
        .get(call_id)
        .is_some_and(|c| c.peer_pubkey == sender_hex && matches!(c.status, CallStatus::Outgoing));
    if !known {
        return;
    }
    deps.emit_event(CallSignalEvent::CallRinging {
        call_id: call_id.to_string(),
        peer_public_key: sender_hex.to_string(),
    });
}

/// Helper — fire a `CallDecline` at a peer without inserting any
/// CallState (used for malformed-invite + temp-mute rejections).
async fn send_decline<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    peer_pubkey: &str,
    call_id: &str,
    reason: &str,
) {
    let payload = MessagePayload::CallDecline {
        call_id: call_id.to_string(),
        reason: reason.to_string(),
    };
    if let Err(e) = deps.send_to_peer(peer_pubkey, payload).await {
        tracing::debug!(call = %call_id, error = %e, "CallDecline send failed");
    }
}

/// W13.15 helper — cancel an Outgoing call we lost to glare. Sends
/// `CallEnd` to the peer, removes our CallState, tears down voice if
/// it was active, and emits `CallEnded` so the frontend clears the
/// outgoing slot.
async fn cancel_outgoing_for_glare<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    peer_pubkey: &str,
) {
    // W15.4 — capture status BEFORE the remove so we know whether
    // voice was running. If it was, tear it down so the loser's mic
    // and speakers actually stop.
    let was_voice_up = deps
        .registry()
        .remove(call_id)
        .is_some_and(|c| matches!(c.status, CallStatus::Active | CallStatus::Connecting));
    if was_voice_up {
        deps.shutdown_voice_session().await;
    }
    let payload = MessagePayload::CallEnd {
        call_id: call_id.to_string(),
        reason: "glare-resolved-we-lost".into(),
    };
    if let Err(e) = deps.send_to_peer(peer_pubkey, payload).await {
        tracing::debug!(call = %call_id, error = %e, "CallEnd (glare) send failed");
    }
    deps.emit_event(CallSignalEvent::CallEnded {
        call_id: call_id.to_string(),
        peer_public_key: peer_pubkey.to_string(),
        reason: "glare-resolved".into(),
        voice_was_up: was_voice_up,
    });
}
