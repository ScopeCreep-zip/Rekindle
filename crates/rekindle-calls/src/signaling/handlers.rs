//! Phase 14 — 1:1 call signaling handlers.
//!
//! Receive arms for `CallInvite`/`CallAccept`/`CallDecline`/`CallRinging`
//! (architecture §10.10, Wave 13). Parameterized over
//! [`CallSignalingDeps`] so this crate stays free of `AppState`,
//! `tauri::AppHandle`, and direct `veilid-core` references. The
//! src-tauri adapter (lands in 14.h) implements the deps trait against
//! `AppState` + the transport's `operations::dm::send_to_peer` + `services::voice::*`.
//!
//! Pre-Phase-14 these lived in `src-tauri/services/calls/mod.rs`.

use rekindle_codec::capnp_codec::direct::{CallClosed, CallRinging, DirectBody};

use crate::signaling::deps::{peer_shown_name, CallSignalingDeps};
use crate::signaling::event::CallSignalEvent;
use crate::state::{CallKind, CallState, CallStatus};

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
        initiator_x25519_pub,
        expires_at_ms,
        video_decode_codecs,
    } = invite;
    let kind = CallKind::from_u8(offer_kind).unwrap_or(CallKind::Audio);
    let Ok(display_name) = peer_shown_name(deps, sender_hex) else {
        tracing::warn!(call = %call_id, "CallInvite with malformed sender key");
        return;
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

    let registry = deps.registry();
    if !make_room_for_invite(deps, sender_hex, call_id).await {
        return;
    }

    // W13.15 glare resolution.
    let our_pubkey = deps.owner_key().unwrap_or_default();
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
        media_secret: None,
        media_sender: std::sync::Arc::new(rekindle_secrets::sframe::SframeSender::fresh()),
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
    send_ringing(deps, sender_hex, call_id).await;

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

    let Ok(display_name) = peer_shown_name(deps, &peer_pubkey) else {
        tracing::warn!(call = %call_id, "CallAccept with malformed sender key");
        return;
    };

    // Plan CE3: the answer settles our invite's outbox row.
    deps.settle_call_invite(call_id, true).await;

    // Plan CE2 (30): the peer answering this call has left any other call
    // with us; end it (and its media) before this call's media starts. It
    // awaits only when such a call exists, so on the usual path nothing
    // awaits before the pre-stage below.
    crate::signaling::lifecycle::supersede_stale_with(deps, &peer_pubkey, call_id).await;

    // W14.1 — pre-stage the voice receive channel BEFORE any other
    // await so packets arriving during the rest of the accept handler
    // buffer in the channel rather than getting dropped at dispatch.
    // Adapter is responsible for the actual `voice_packet_tx/rx_staged`
    // mpsc setup on AppState.
    deps.pre_stage_voice_channel();

    // Derive call_key. On failure, drop the call + emit CallEnded.
    let call_key = match crate::derive_call_key(&my_secret, acceptor_x25519_pub, call_id) {
        Ok(k) => k,
        Err(error) => {
            tracing::debug!(call = %call_id, %error, "could not derive call key");
            registry.remove(call_id);
            deps.emit_event(CallSignalEvent::CallEnded {
                call_id: call_id.to_string(),
                peer_public_key: peer_pubkey,
                peer_reason: None,
                voice_was_up: false,
            });
            return;
        }
    };

    // Update registry: store the media secret + transition to Connecting.
    if let Some(mut call) = registry.get(call_id) {
        call.media_secret = Some(zeroize::Zeroizing::new(call_key));
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
    if let Err(e) = deps.start_voice_session(call_id, &peer_pubkey, kind).await {
        registry.remove(call_id);
        deps.shutdown_voice_session(call_id).await;
        let reason = format!("voice session failed: {e}");
        crate::signaling::lifecycle::commit_end(deps, &peer_pubkey, call_id, &reason).await;
        deps.emit_event(CallSignalEvent::CallEnded {
            call_id: call_id.to_string(),
            peer_public_key: peer_pubkey,
            peer_reason: None,
            voice_was_up: false,
        });
        return;
    }

    // Promote to Active + emit CallConnected + focus chat.
    if let Some(mut call) = registry.get(call_id) {
        call.status = CallStatus::Active;
        registry.insert(call);
    }
    deps.spawn_consent_watch(call_id.to_string(), deps.now_ms());
    deps.emit_event(CallSignalEvent::CallConnected {
        call_id: call_id.to_string(),
        peer_public_key: peer_pubkey.clone(),
        peer_display_name: display_name.clone(),
        kind,
    });
    deps.present_active_call(call_id);
    deps.emit_event(CallSignalEvent::ConversationFocusRequested {
        peer_public_key: peer_pubkey,
        peer_display_name: display_name,
        reason: "call-accepted".to_string(),
    });
}

/// Receive arm for `CallDecline` (W13.8). Caller-side. Idempotent;
/// silently no-ops on unknown call_id or wrong peer.
pub async fn handle_decline_received<D: CallSignalingDeps + ?Sized>(
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
        deps.settle_call_invite(call_id, true).await;
        deps.emit_event(CallSignalEvent::CallDeclined {
            call_id: call_id.to_string(),
            peer_public_key: sender_hex.to_string(),
            peer_reason: reason,
        });
    }
}

/// Receive arm for `CallRinging`. Caller-side alerting hint — receiver
/// has the invite and is ringing. Verifies the ringing is for a call
/// we actually own (defends against a forged ringing for someone else's
/// call_id).
pub async fn handle_ringing_received<D: CallSignalingDeps + ?Sized>(
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
    // Plan CE3: the callee holds the invite; its row stops re-sending.
    deps.settle_call_invite(call_id, true).await;
    deps.emit_event(CallSignalEvent::CallRinging {
        call_id: call_id.to_string(),
        peer_public_key: sender_hex.to_string(),
    });
}

/// Receive arm for `CallEnd` (W13.9): the peer hung up or cancelled, in
/// any state. Idempotent: a `CallEnd` for a call we no longer hold, or
/// from another peer, is a no-op. Only `call_id`'s media is torn down
/// (plan CE2, lead decision 30): a late `CallEnd` for a superseded call
/// never stops the new call's media.
pub async fn handle_end_received<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
    reason: String,
) {
    let registry = deps.registry();
    let ours = registry
        .get(call_id)
        .is_some_and(|c| c.peer_pubkey == sender_hex);
    let Some(removed) = ours.then(|| registry.remove(call_id)).flatten() else {
        tracing::debug!(call = %call_id, "CallEnd for a call we do not hold; ignoring");
        return;
    };
    let was_voice_up = matches!(removed.status, CallStatus::Active | CallStatus::Connecting);
    drop(removed); // Zeroize the call's secrets now.
    if was_voice_up {
        deps.shutdown_voice_session(call_id).await;
    }
    tracing::info!(call = %call_id, %reason, "remote ended DM call");
    deps.emit_event(CallSignalEvent::CallEnded {
        call_id: call_id.to_string(),
        peer_public_key: sender_hex.to_string(),
        peer_reason: (!reason.is_empty()).then_some(reason),
        voice_was_up: was_voice_up,
    });
}

/// Plan CE1, decision 22 (c): whether an invite for `call_id` from
/// `sender_hex` may ring. A repeated invite for a call we hold does not
/// ring again. A call with the same peer under another call id (Incoming,
/// Connecting or Active) is stale, since the peer has left it, and is ended
/// here first. A Connecting or Active call with another peer answers this
/// invite busy. Our own Outgoing call to the sender is glare's, after this.
async fn make_room_for_invite<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    call_id: &str,
) -> bool {
    let registry = deps.registry();
    if let Some(held) = registry.get(call_id) {
        tracing::debug!(call = %call_id, "repeated CallInvite for a call we hold; not ringing again");
        if held.peer_pubkey == sender_hex && matches!(held.status, CallStatus::Incoming) {
            // Plan CE3: our first CallRinging may be lost; the caller's
            // outbox re-sends the invite until one arrives.
            send_ringing(deps, sender_hex, call_id).await;
        }
        return false;
    }
    if registry.ended_recently(call_id) {
        tracing::debug!(call = %call_id, "re-sent CallInvite for a call that ended; not ringing");
        return false;
    }
    for call in registry.list_all() {
        let up = matches!(call.status, CallStatus::Active | CallStatus::Connecting);
        if call.peer_pubkey != sender_hex && up {
            send_decline(deps, sender_hex, call_id, BUSY).await;
            return false;
        }
    }
    crate::signaling::lifecycle::supersede_stale_with(deps, sender_hex, call_id).await;
    true
}

/// Best-effort `CallRinging` to the caller: it settles their invite's
/// outbox row (plan CE3) and flips their UI to "Ringing…".
async fn send_ringing<D: CallSignalingDeps + ?Sized>(deps: &D, caller: &str, call_id: &str) {
    let ringing = DirectBody::CallRinging(CallRinging {
        call_id: call_id.to_string(),
    });
    if let Err(e) = deps.send_to_peer(caller, ringing).await {
        tracing::debug!(call = %call_id, error = %e, "CallRinging ack send failed");
    }
}

/// The decline reason an invite gets while we are in another call.
pub const BUSY: &str = "busy";

/// Helper — fire a `CallDecline` at a peer without inserting any
/// CallState (used for malformed-invite + temp-mute rejections).
async fn send_decline<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    peer_pubkey: &str,
    call_id: &str,
    reason: &str,
) {
    let payload = DirectBody::CallDecline(CallClosed {
        call_id: call_id.to_string(),
        reason: reason.to_string(),
    });
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
        deps.shutdown_voice_session(call_id).await;
    }
    deps.settle_call_invite(call_id, false).await;
    crate::signaling::lifecycle::commit_end(deps, peer_pubkey, call_id, "glare-resolved-we-lost")
        .await;
    deps.emit_event(CallSignalEvent::CallEnded {
        call_id: call_id.to_string(),
        peer_public_key: peer_pubkey.to_string(),
        peer_reason: None,
        voice_was_up: was_voice_up,
    });
}
