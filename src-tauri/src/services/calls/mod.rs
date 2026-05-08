//! Wave 13 — direct-call signaling state machine.
//!
//! Replaces the old `services/call_signaling.rs` (which was a
//! pending-accept oneshot table — workaround for routing CallOffer
//! over Veilid `app_call`). Wave 13 flips the carrier to fire-and-forget
//! `app_message`, mirroring the friend-add handshake that already works.
//!
//! Five message types travel over `app_message`:
//!   - `CallInvite`   — caller → callee  (fired by `commands::calls::start_dm_call`)
//!   - `CallRinging`  — callee → caller  (optional alerting ack from `handle_incoming_invite`)
//!   - `CallAccept`   — callee → caller  (fired by `commands::calls::accept_dm_call`)
//!   - `CallDecline`  — callee → caller  (fired by `commands::calls::decline_dm_call`)
//!   - `CallEnd`      — either party     (fired by `commands::calls::end_dm_call`)
//!
//! Each side enforces the 30 s ring timeout independently via the
//! `ring_timer` submodule. No oneshot. No RPC.
//!
//! Reference patterns: SimpleX `x.call.inv` / `x.call.offer` / `x.call.answer`
//! / `x.call.end` (see plan W13 research). Signal `CallMessage::Offer`
//! / `Answer` / `Hangup`. Both fire-and-forget; both proven over
//! comparable P2P transports.

pub mod ring_timer;

use rekindle_calls::{derive_call_key, CallKind, CallState, CallStatus};
use rekindle_protocol::messaging::envelope::MessagePayload;
use tauri::Emitter;

use crate::channels::{ChatEvent, NotificationEvent};
use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;

/// Persist a missed-call row (kept in the calls module so the ring
/// timer can call into it without crossing module boundaries).
pub(crate) fn persist_missed_call(
    pool: &DbPool,
    state: &SharedState,
    call_id: &str,
    peer_key: &str,
    kind: CallKind,
    expired_at_ms: u64,
) {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let cid = call_id.to_string();
    let pk = peer_key.to_string();
    let kind_u8 = i64::from(kind.as_u8());
    let expired = i64::try_from(expired_at_ms).unwrap_or(i64::MAX);
    crate::db_helpers::db_fire(pool, "persist missed call", move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO missed_calls (call_id, owner_key, peer_key, kind, expired_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![cid, owner_key, pk, kind_u8, expired],
        )?;
        Ok(())
    });
}

/// Truncate a hex pubkey for display when no friend display name is
/// known.
fn short_pubkey(pk: &str) -> String {
    if pk.len() > 16 {
        format!("{}…", &pk[..16])
    } else {
        pk.to_string()
    }
}

/// W13.4 — receive arm for `CallInvite`. Receiver-side entry into the
/// state machine. Mirrors `handle_friend_request_full` shape: persist
/// (in-memory CallState here, not SQLite — calls aren't durable like
/// friend requests), emit `chat-event`, surface window, schedule the
/// 30 s ring timeout. Fire-and-forget `CallRinging` ack to the caller
/// so their UI can flip from "Calling…" to "Ringing…".
///
/// Returns immediately; user accept/decline runs as a separate IPC
/// command later (`commands::calls::accept_dm_call` /
/// `decline_dm_call`).
pub async fn handle_incoming_invite(
    app: &tauri::AppHandle,
    state: &SharedState,
    pool: &DbPool,
    sender_hex: &str,
    call_id: &str,
    offer_kind: u8,
    initiator_pubkey: &str,
    initiator_x25519_pub: &[u8],
    expires_at_ms: u64,
) {
    let kind = CallKind::from_u8(offer_kind).unwrap_or(CallKind::Audio);
    let display_name = state_helpers::friend_display_name(state, sender_hex)
        .unwrap_or_else(|| short_pubkey(initiator_pubkey));

    if initiator_x25519_pub.len() != 32 {
        // Malformed invite — fire Decline and drop.
        send_decline(state, pool, sender_hex, call_id, "invalid x25519 public key").await;
        return;
    }
    let mut peer_arr = [0u8; 32];
    peer_arr.copy_from_slice(initiator_x25519_pub);

    // W12.12 temp-mute: silently auto-decline without ringing.
    let temp_muted = {
        let now = rekindle_utils::timestamp_ms();
        let mut muted = state.temp_call_muted.lock();
        match muted.get(sender_hex).copied() {
            Some(expires_at) if now < expires_at => true,
            Some(_) => {
                muted.remove(sender_hex);
                false
            }
            None => false,
        }
    };
    if temp_muted {
        send_decline(state, pool, sender_hex, call_id, "user is unavailable").await;
        return;
    }

    // W13.15 glare resolution: if we have an Outgoing call to the SAME
    // peer in flight, the side with the LOWER hex pubkey wins their
    // outgoing (deterministic — both sides reach the same conclusion).
    // The loser cancels their Outgoing and processes this Incoming.
    let our_pubkey = state_helpers::current_identity(state)
        .map(|i| i.public_key)
        .unwrap_or_default();
    let glare_ids: Vec<String> = {
        let calls = state.active_calls.lock();
        calls
            .iter()
            .filter(|(_, c)| {
                c.peer_pubkey == sender_hex
                    && matches!(c.status, CallStatus::Outgoing)
            })
            .map(|(id, _)| id.clone())
            .collect()
    };
    if !glare_ids.is_empty() {
        let we_win = our_pubkey.as_str() < sender_hex;
        if we_win {
            // We win — auto-decline this invite; our outgoing continues.
            send_decline(state, pool, sender_hex, call_id, "glare-resolved-other-loses").await;
            return;
        }
        // We lose. Cancel our outgoing(s) for this peer.
        for cid in glare_ids {
            cancel_outgoing_for_glare(state, pool, &cid, sender_hex, app).await;
        }
    }

    // Insert CallState as Incoming. Receiver's local X25519 keypair is
    // generated at accept-time (in commands::calls::accept_dm_call) —
    // not here, because if the user declines we don't want to have
    // generated a key for nothing.
    {
        let mut calls = state.active_calls.lock();
        calls.insert(
            call_id.to_string(),
            CallState {
                call_id: call_id.to_string(),
                peer_pubkey: sender_hex.to_string(),
                kind,
                status: CallStatus::Incoming,
                expires_at_ms,
                my_x25519_secret: None,
                peer_x25519_pub: Some(peer_arr),
                call_key: None,
            },
        );
    }

    // W13-fix surface: the Wave 12 chat-event::IncomingCall and
    // notification-event::CallIncoming pair plus W12-fix.C window
    // surface. All three fire AFTER the CallState insert so a fast
    // user-accept can find the entry.
    let kind_str = match kind {
        CallKind::Audio => "audio",
        CallKind::Video => "video",
    };
    let _ = app.emit(
        "chat-event",
        &ChatEvent::IncomingCall {
            call_id: call_id.to_string(),
            from: sender_hex.to_string(),
            display_name: display_name.clone(),
            kind: kind_str.into(),
            expires_at_ms,
        },
    );
    let _ = app.emit(
        "notification-event",
        &NotificationEvent::CallIncoming {
            call_id: call_id.to_string(),
            from: sender_hex.to_string(),
            display_name,
            kind: kind_str.into(),
            expires_at_ms,
            is_group: false,
        },
    );
    crate::windows::surface_window_for_call(app);

    // Best-effort CallRinging ack so the caller's UI can flip
    // "Calling…" → "Ringing…". Failure is fine (the caller won't see
    // the transition but will still see CallAccept / CallDecline /
    // CallTimedOut as definitive states).
    let ringing = MessagePayload::CallRinging {
        call_id: call_id.to_string(),
    };
    let _ = crate::services::message_service::send_to_peer_raw(
        state, pool, sender_hex, &ringing,
    )
    .await;

    // W13.2 — schedule local 30 s timeout. Independent of the caller's
    // timeout (each side enforces its own).
    ring_timer::spawn_incoming_timeout(
        state.clone(),
        pool.clone(),
        app.clone(),
        call_id.to_string(),
        sender_hex.to_string(),
        kind,
        expires_at_ms,
    );
}

/// W13.6 — receive arm for `CallAccept`. Caller-side: receiver has
/// accepted; derive the shared `call_key`, start the voice session,
/// transition to Active. If voice session fails, send a CallEnd back
/// so the receiver's just-started session tears down.
///
/// W14.1 — IMMEDIATELY upon CallAccept arrival, pre-create the voice
/// packet mpsc channel and stash both ends on AppState. This way the
/// app_message dispatcher (services/veilid/app_message.rs) can route
/// inbound voice packets into the buffer before start_session
/// finishes — eliminating the dispatch race where the receiver's first
/// audio frames arrived during start_session setup and were dropped.
pub async fn handle_accept_received(
    app: &tauri::AppHandle,
    state: &SharedState,
    pool: &DbPool,
    sender_hex: &str,
    call_id: &str,
    acceptor_x25519_pub: &[u8],
) {
    if acceptor_x25519_pub.len() != 32 {
        tracing::warn!(call = %call_id, "CallAccept with bad x25519 length");
        return;
    }

    // W14.1 — pre-stage the voice receive channel BEFORE any await
    // points so packets arriving during the rest of the accept handler
    // buffer in the channel rather than getting dropped at dispatch.
    {
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *state.voice_packet_tx.write() = Some(tx);
        *state.voice_packet_rx_staged.lock() = Some(rx);
        tracing::info!(call = %call_id,
            "W14.1 — pre-staged voice receive channel on CallAccept arrival");
    }

    // Take the secret + verify state matches.
    let (my_secret, peer_pubkey) = {
        let mut calls = state.active_calls.lock();
        let Some(call) = calls.get_mut(call_id) else {
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
        let Some(secret) = call.my_x25519_secret.take() else {
            tracing::warn!(call = %call_id, "CallAccept but local x25519 secret missing");
            return;
        };
        let mut peer_arr = [0u8; 32];
        peer_arr.copy_from_slice(acceptor_x25519_pub);
        call.peer_x25519_pub = Some(peer_arr);
        call.status = CallStatus::Connecting;
        (secret, call.peer_pubkey.clone())
    };

    // Derive call_key.
    let call_key = match derive_call_key(&my_secret, acceptor_x25519_pub, call_id) {
        Ok(k) => k,
        Err(e) => {
            state.active_calls.lock().remove(call_id);
            let _ = app.emit(
                "chat-event",
                &ChatEvent::CallEnded {
                    call_id: call_id.to_string(),
                    reason: format!("derive call_key: {e}"),
                },
            );
            return;
        }
    };
    {
        let mut calls = state.active_calls.lock();
        if let Some(call) = calls.get_mut(call_id) {
            call.call_key = Some(call_key);
        }
    }

    // Bring up our voice session. If it fails, send CallEnd so the
    // receiver's just-started session tears down.
    if let Err(e) =
        crate::services::voice::session::start_session(&peer_pubkey, None, app, state).await
    {
        state.active_calls.lock().remove(call_id);
        let hangup = MessagePayload::CallEnd {
            call_id: call_id.to_string(),
            reason: format!("voice session failed: {e}"),
        };
        let _ = crate::services::message_service::send_to_peer_raw(
            state, pool, &peer_pubkey, &hangup,
        )
        .await;
        let _ = app.emit(
            "chat-event",
            &ChatEvent::CallEnded {
                call_id: call_id.to_string(),
                reason: format!("voice session failed: {e}"),
            },
        );
        return;
    }

    // Promote to Active and emit CallConnected with W14.2 payload.
    let (kind_str, expected_local_camera) = {
        let mut calls = state.active_calls.lock();
        match calls.get_mut(call_id) {
            Some(call) => {
                call.status = CallStatus::Active;
                let kind_str = match call.kind {
                    CallKind::Audio => "audio",
                    CallKind::Video => "video",
                };
                (kind_str.to_string(), matches!(call.kind, CallKind::Video))
            }
            None => ("audio".to_string(), false),
        }
    };
    let peer_display_name = state_helpers::friend_display_name(state, &peer_pubkey)
        .unwrap_or_else(|| short_pubkey(&peer_pubkey));
    let _ = app.emit(
        "chat-event",
        &ChatEvent::CallConnected {
            call_id: call_id.to_string(),
            kind: kind_str,
            peer_key: peer_pubkey.clone(),
            peer_display_name: peer_display_name.clone(),
            expected_local_camera,
        },
    );
    // W14.3 — focus the conversation on the caller side too. Receiver
    // already focused on accept; caller focuses now that voice is up.
    let _ = app.emit(
        "chat-event",
        &ChatEvent::ConversationFocusRequested {
            peer_key: peer_pubkey,
            display_name: peer_display_name,
            reason: "call-accepted".into(),
        },
    );
}

/// W13.8 — receive arm for `CallDecline`. Caller-side: receiver
/// rejected the call. Drop the CallState and emit CallDeclined.
/// Idempotent — silently no-ops on unknown call_id.
pub async fn handle_decline_received(
    app: &tauri::AppHandle,
    state: &SharedState,
    sender_hex: &str,
    call_id: &str,
    reason: String,
) {
    let removed = {
        let mut calls = state.active_calls.lock();
        match calls.get(call_id) {
            Some(c) if c.peer_pubkey == sender_hex => calls.remove(call_id).is_some(),
            _ => false,
        }
    };
    if removed {
        let _ = app.emit(
            "chat-event",
            &ChatEvent::CallDeclined {
                call_id: call_id.to_string(),
                reason,
            },
        );
    }
}

/// Receive arm for `CallRinging`. Caller-side alerting hint — receiver
/// has the invite and is ringing the user. Emits a chat-event the
/// frontend can use to flip the OutgoingCallPanel label from
/// "Calling…" to "Ringing…".
pub async fn handle_ringing_received(
    app: &tauri::AppHandle,
    state: &SharedState,
    sender_hex: &str,
    call_id: &str,
) {
    // Verify the ringing is for a call we actually own (defends
    // against a forged ringing for someone else's call_id).
    let known = {
        let calls = state.active_calls.lock();
        calls
            .get(call_id)
            .is_some_and(|c| c.peer_pubkey == sender_hex && matches!(c.status, CallStatus::Outgoing))
    };
    if !known {
        return;
    }
    let _ = app.emit(
        "chat-event",
        &ChatEvent::CallRinging {
            call_id: call_id.to_string(),
        },
    );
}

/// Helper — fire a CallDecline at a peer without inserting any
/// CallState (used for malformed-invite and temp-mute rejections).
async fn send_decline(
    state: &SharedState,
    pool: &DbPool,
    peer_pubkey: &str,
    call_id: &str,
    reason: &str,
) {
    let payload = MessagePayload::CallDecline {
        call_id: call_id.to_string(),
        reason: reason.to_string(),
    };
    let _ = crate::services::message_service::send_to_peer_raw(
        state, pool, peer_pubkey, &payload,
    )
    .await;
}

/// W13.15 helper — cancel an Outgoing call we lose to glare. Sends
/// CallEnd to the peer (so they cancel their just-arrived
/// CallInvite-side state if any), removes our CallState, and clears
/// our frontend's outgoing slot via CallEnded.
async fn cancel_outgoing_for_glare(
    state: &SharedState,
    pool: &DbPool,
    call_id: &str,
    peer_pubkey: &str,
    app: &tauri::AppHandle,
) {
    state.active_calls.lock().remove(call_id);
    let payload = MessagePayload::CallEnd {
        call_id: call_id.to_string(),
        reason: "glare-resolved-we-lost".into(),
    };
    let _ = crate::services::message_service::send_to_peer_raw(
        state, pool, peer_pubkey, &payload,
    )
    .await;
    let _ = app.emit(
        "chat-event",
        &ChatEvent::CallEnded {
            call_id: call_id.to_string(),
            reason: "glare-resolved".into(),
        },
    );
}
