//! Plan §Failure 5 / Architecture §10.10 — direct call signalling.
//!
//! Three Tauri commands implement the offer/accept handshake on top of
//! Veilid `app_call`:
//!
//! * [`start_dm_call`]   — caller side: generates `call_id` + ephemeral
//!   X25519, ships `CallOffer`, awaits the inline `CallAccept` /
//!   `CallDecline` reply, on accept derives `call_key` and starts the
//!   voice transport, on decline / timeout writes a `missed_calls` row.
//! * [`accept_dm_call`]  — callee side: looks up the pending Incoming
//!   call, generates its own X25519 keypair, derives `call_key`, opens
//!   the voice transport, and returns the accept reply via the
//!   inbound-dispatch oneshot wired up in `message_service`.
//! * [`decline_dm_call`] — callee side: same lookup, returns the
//!   decline reply.
//!
//! All three sit on top of `services::message_service::send_to_peer_call`
//! (caller) and the inbound `app_call` reply path (callee). No new
//! transport is introduced — this is the same shape as DM invite +
//! relay-offer flows.

use std::time::Duration;

use rand::RngCore;
use rusqlite::params;
use tauri::{Emitter, State};

use rekindle_calls::{derive_call_key, fresh_keypair, CallKind, CallState, CallStatus};
use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::channels::{ChatEvent, NotificationEvent};
use crate::db::DbPool;
use crate::db_helpers::{db_call, db_fire};
use crate::state::SharedState;
use crate::state_helpers;

/// Architecture §10.10 — 30 s ring before the call is logged as missed.
const RING_DURATION_MS: u64 = 30_000;

#[tauri::command]
pub async fn start_dm_call(
    peer_public_key: String,
    video: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let call_id = generate_call_id();
    let kind = if video {
        CallKind::Video
    } else {
        CallKind::Audio
    };
    let (my_secret, my_pub) = fresh_keypair();
    let initiator_pubkey = state_helpers::current_identity(state.inner())
        .map_err(|_| "identity not set".to_string())?
        .public_key;
    let expires_at_ms = rekindle_utils::timestamp_ms() + RING_DURATION_MS;

    {
        let mut calls = state.active_calls.lock();
        calls.insert(
            call_id.clone(),
            CallState {
                call_id: call_id.clone(),
                peer_pubkey: peer_public_key.clone(),
                kind,
                status: CallStatus::Outgoing,
                expires_at_ms,
                my_x25519_secret: Some(my_secret),
                peer_x25519_pub: None,
                call_key: None,
            },
        );
    }

    // Spawn the ring timer first so a slow `app_call` can still fire
    // the timeout. Architecture §10.10 — 30 s ring across all kinds.
    let timeout_state = state.inner().clone();
    let timeout_pool = pool.inner().clone();
    let timeout_app = app.clone();
    let timeout_call_id = call_id.clone();
    let timeout_peer = peer_public_key.clone();
    state
        .background_handles
        .lock()
        .push(tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(RING_DURATION_MS)).await;
            let still_pending = {
                let calls = timeout_state.active_calls.lock();
                calls
                    .get(&timeout_call_id)
                    .is_some_and(|c| matches!(c.status, CallStatus::Outgoing))
            };
            if !still_pending {
                return;
            }
            timeout_state.active_calls.lock().remove(&timeout_call_id);
            persist_missed_call(
                &timeout_pool,
                &timeout_state,
                &timeout_call_id,
                &timeout_peer,
                kind,
                expires_at_ms,
            );
            let _ = timeout_app.emit(
                "chat-event",
                &ChatEvent::CallTimedOut {
                    call_id: timeout_call_id,
                },
            );
        }));

    let offer = MessagePayload::CallOffer {
        call_id: call_id.clone(),
        offer_kind: kind.as_u8(),
        initiator_pubkey: initiator_pubkey.clone(),
        initiator_x25519_pub: my_pub.to_vec(),
        expires_at_ms,
    };

    let reply =
        crate::services::message_service::send_to_peer_call(state.inner(), &peer_public_key, &offer)
            .await;

    match reply {
        Ok(MessagePayload::CallAccept {
            call_id: reply_call_id,
            acceptor_x25519_pub,
        }) if reply_call_id == call_id => {
            finalize_outgoing_accept(
                &app,
                state.inner(),
                &call_id,
                &peer_public_key,
                &acceptor_x25519_pub,
            )
            .await?;
            Ok(call_id)
        }
        Ok(MessagePayload::CallDecline { reason, .. }) => {
            state.active_calls.lock().remove(&call_id);
            let _ = app.emit(
                "chat-event",
                &ChatEvent::CallDeclined {
                    call_id: call_id.clone(),
                    reason: reason.clone(),
                },
            );
            Err(if reason.is_empty() {
                "call declined".into()
            } else {
                format!("call declined: {reason}")
            })
        }
        Ok(other) => {
            state.active_calls.lock().remove(&call_id);
            Err(format!("unexpected reply to CallOffer: {other:?}"))
        }
        Err(e) => {
            state.active_calls.lock().remove(&call_id);
            Err(format!("call offer send failed: {e}"))
        }
    }
}

#[tauri::command]
pub async fn accept_dm_call(
    call_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    // Frontend invokes this to consent. The actual `CallAccept` reply
    // travels via `prepare_call_accept_reply` (see message_service)
    // because the responder must reply on the same `app_call` socket
    // the caller opened. Here we only flip the `CallState` so the
    // inbound handler picks Accept, and signal the waiting oneshot.
    let tx = {
        let mut calls = state.active_calls.lock();
        let call = calls
            .get_mut(&call_id)
            .ok_or("no incoming call with that id")?;
        if !matches!(call.status, CallStatus::Incoming) {
            return Err("call is not in Incoming state".into());
        }
        call.status = CallStatus::Active;
        crate::services::call_signaling::take_pending_response(&call_id)
    };
    let tx = tx.ok_or("no pending accept handshake — call already resolved")?;
    let _ = tx.send(crate::services::call_signaling::IncomingDecision::Accept);
    Ok(())
}

/// C2 hangup — end an Active call (post-handshake). Removes from
/// active_calls, sends a CallEnd payload to the peer so they also clear
/// their state, and emits ChatEvent::CallEnded locally so the frontend's
/// callsState.activeCall slot clears.
///
/// Distinct from `decline_dm_call` (which is the user rejecting an
/// inbound CallOffer before accepting). `end_dm_call` is the user
/// hanging up an already-connected call.
#[tauri::command]
pub async fn end_dm_call(
    call_id: String,
    reason: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let peer_pubkey = {
        let mut calls = state.active_calls.lock();
        let call = calls
            .remove(&call_id)
            .ok_or_else(|| format!("no active call with id {call_id}"))?;
        // CallState implements Drop (zeroizing the X25519 secret), so we
        // can't move fields out partially. Clone the pubkey String, then
        // let the rest of `call` drop normally — its zeroize-on-drop runs
        // and the X25519 secret is wiped from memory.
        call.peer_pubkey.clone()
    };

    let reason_str = reason.unwrap_or_default();

    // Notify the peer their CallState is now stale. Best-effort —
    // app_message may fail if the peer just dropped offline; the
    // 30-second presence-poll on their side will eventually GC the
    // stale call entry. We don't return an error here because the
    // local-side hangup must succeed regardless of peer reachability.
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::CallEnd {
        call_id: call_id.clone(),
        reason: reason_str.clone(),
    };
    if let Err(e) = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_pubkey,
        &payload,
    )
    .await
    {
        tracing::info!(
            call_id = %call_id,
            peer = %peer_pubkey,
            error = %e,
            "CallEnd send to peer failed; their state will GC on next poll"
        );
    }

    // Emit local CallEnded so the frontend clears callsState.activeCall.
    let _ = app.emit(
        "chat-event",
        &crate::channels::ChatEvent::CallEnded {
            call_id: call_id.clone(),
            reason: reason_str,
        },
    );
    Ok(())
}

/// Wave 12 W12.9 — receive side. Mirrors `handle_incoming_offer` but
/// for groups: unwraps the per-recipient call_key, creates the local
/// GroupCallState, emits ChatEvent::IncomingGroupCall, then parks on a
/// oneshot for the user's accept/decline. Returns the inline app_call
/// reply.
pub async fn handle_incoming_group_offer(
    app: &tauri::AppHandle,
    state: &SharedState,
    sender_hex: &str,
    call_id: &str,
    offer_kind: u8,
    initiator_pubkey: &str,
    initiator_x25519_pub: &[u8],
    participants: Vec<String>,
    wrapped_call_key: &[u8],
    expires_at_ms: u64,
) -> MessagePayload {
    use crate::services::group_calls::{unwrap_offer, GroupCallState, GroupCallStatus};

    let kind = CallKind::from_u8(offer_kind).unwrap_or(CallKind::Audio);
    let display_name = state_helpers::friend_display_name(state, sender_hex)
        .unwrap_or_else(|| short_pubkey(initiator_pubkey));

    if initiator_x25519_pub.len() != 32 {
        return MessagePayload::GroupCallDecline {
            call_id: call_id.to_string(),
            reason: "invalid initiator x25519 key".into(),
        };
    }

    // Temp-mute lookup mirrors 1:1 — auto-decline silently.
    {
        let now = rekindle_utils::timestamp_ms();
        let mut muted = state.temp_call_muted.lock();
        if let Some(&expires_at) = muted.get(sender_hex) {
            if now < expires_at {
                return MessagePayload::GroupCallDecline {
                    call_id: call_id.to_string(),
                    reason: "user is unavailable".into(),
                };
            }
            muted.remove(sender_hex);
        }
    }

    // Generate our X25519 keypair so we can be wrapped FOR by the
    // initiator. Per the wrap design, the initiator already wrapped
    // call_key for us using our Ed25519→X25519 conversion; our
    // matching secret is the same conversion of our own Ed25519. We
    // hold an ephemeral pair too for the unwrap path.
    let our_ed_pubkey = match state_helpers::current_identity(state) {
        Ok(i) => i.public_key,
        Err(_) => {
            return MessagePayload::GroupCallDecline {
                call_id: call_id.to_string(),
                reason: "identity not loaded".into(),
            };
        }
    };
    let our_ed_bytes = match hex::decode(&our_ed_pubkey) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => {
            return MessagePayload::GroupCallDecline {
                call_id: call_id.to_string(),
                reason: "bad local identity".into(),
            };
        }
    };
    // Convert our Ed25519 secret to its X25519 equivalent — same
    // birational map the wrap initiator used on our public key.
    let our_x25519_secret = {
        let secret_opt = state.identity_secret.lock().clone();
        match secret_opt {
            Some(bytes) => {
                let identity = rekindle_crypto::Identity::from_secret_bytes(&bytes);
                identity.to_x25519_secret()
            }
            None => {
                return MessagePayload::GroupCallDecline {
                    call_id: call_id.to_string(),
                    reason: "local identity secret missing".into(),
                };
            }
        }
    };
    let call_key = match unwrap_offer(
        &our_x25519_secret,
        initiator_x25519_pub,
        call_id,
        &our_ed_pubkey,
        wrapped_call_key,
    ) {
        Ok(k) => k,
        Err(e) => {
            return MessagePayload::GroupCallDecline {
                call_id: call_id.to_string(),
                reason: format!("unwrap failed: {e}"),
            };
        }
    };
    // Suppress the "unused identity bytes" warning — we rely on it
    // implicitly above via `current_identity`.
    let _ = our_ed_bytes;

    // Insert the GroupCallState in Incoming so the frontend can render
    // the ring banner and route the user's accept/decline through
    // call_signaling (same oneshot pattern as 1:1).
    {
        let mut calls = state.group_calls.lock();
        calls.insert(
            call_id.to_string(),
            GroupCallState {
                call_id: call_id.to_string(),
                initiator_pubkey: initiator_pubkey.to_string(),
                kind: offer_kind,
                participants: participants.clone(),
                accepted: std::collections::HashSet::new(),
                our_x25519_secret: Some(our_x25519_secret),
                call_key: Some(call_key),
                status: GroupCallStatus::Incoming,
            },
        );
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::services::call_signaling::insert_pending_response(call_id, tx);

    let _ = app.emit(
        "chat-event",
        &ChatEvent::IncomingGroupCall {
            call_id: call_id.to_string(),
            from: sender_hex.to_string(),
            display_name: display_name.clone(),
            kind: match kind {
                CallKind::Audio => "audio".into(),
                CallKind::Video => "video".into(),
            },
            participants: participants.clone(),
            expires_at_ms,
        },
    );

    let now = rekindle_utils::timestamp_ms();
    let remaining = expires_at_ms.saturating_sub(now);
    let timeout = Duration::from_millis(remaining.max(1));

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(crate::services::call_signaling::IncomingDecision::Accept)) => {
            // Frontend has populated GroupCallState as Active in
            // accept_group_call.
            MessagePayload::GroupCallAccept {
                call_id: call_id.to_string(),
                acceptor_pubkey: our_ed_pubkey,
            }
        }
        Ok(Ok(crate::services::call_signaling::IncomingDecision::Decline(reason))) => {
            state.group_calls.lock().remove(call_id);
            MessagePayload::GroupCallDecline {
                call_id: call_id.to_string(),
                reason,
            }
        }
        Ok(Err(_)) | Err(_) => {
            crate::services::call_signaling::take_pending_response(call_id);
            state.group_calls.lock().remove(call_id);
            let _ = app.emit(
                "chat-event",
                &ChatEvent::GroupCallEnded {
                    call_id: call_id.to_string(),
                    reason: "ring expired".into(),
                },
            );
            MessagePayload::GroupCallDecline {
                call_id: call_id.to_string(),
                reason: "ring expired".into(),
            }
        }
    }
}

/// Wave 12 W12.9 — start a group call. Generates a fresh shared
/// `call_key`, then for each invitee derives a per-recipient wrap via
/// X25519 + HKDF + AES-256-GCM and sends a `GroupCallOffer` envelope
/// (one per recipient — each carries only their own wrap, scoping the
/// key strictly to the explicit invite list).
///
/// Returns the `call_id` synchronously once all offers have been
/// dispatched. Replies arrive as background tasks because we don't
/// want to block the caller on every invitee's accept/decline; they
/// surface via ChatEvent::GroupCallParticipantJoined / Left as they
/// resolve. The local user's own slot transitions to Active on the
/// first accept (Discord parity — calling rings everyone, but the
/// initiator can talk to whoever picked up first).
#[tauri::command]
pub async fn start_group_call(
    participant_pubkeys: Vec<String>,
    video: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<String, String> {
    use rekindle_calls::group::{generate_call_key, wrap_call_key};
    use crate::services::group_calls::{GroupCallState, GroupCallStatus};

    if participant_pubkeys.is_empty() {
        return Err("group call requires at least one invitee".into());
    }

    let call_id = generate_call_id();
    let kind = if video { CallKind::Video } else { CallKind::Audio };
    let (my_secret, my_pub) = fresh_keypair();
    let initiator_pubkey = state_helpers::current_identity(state.inner())
        .map_err(|_| "identity not set".to_string())?
        .public_key;
    let expires_at_ms = rekindle_utils::timestamp_ms() + RING_DURATION_MS;
    let call_key = generate_call_key();

    // Build the full participants list (initiator + invitees).
    let mut all_participants = vec![initiator_pubkey.clone()];
    for pk in &participant_pubkeys {
        if !all_participants.contains(pk) {
            all_participants.push(pk.clone());
        }
    }

    // Pre-compute each invitee's wrap. We need their X25519 public for
    // ECDH; for now we re-use the initiator's identity as a stand-in
    // because group calls between fresh strangers require a separate
    // prekey-bundle fetch. Per-recipient wraps still scope the key.
    // TODO once strand_relay integrates: pull each recipient's
    // long-term X25519 from their PreKeyBundle on the DHT.
    {
        let mut calls = state.group_calls.lock();
        calls.insert(
            call_id.clone(),
            GroupCallState {
                call_id: call_id.clone(),
                initiator_pubkey: initiator_pubkey.clone(),
                kind: kind.as_u8(),
                participants: all_participants.clone(),
                accepted: std::collections::HashSet::new(),
                our_x25519_secret: Some(my_secret),
                call_key: Some(call_key),
                status: GroupCallStatus::Outgoing,
            },
        );
    }

    // Fan out — one app_call per invitee, each carrying THEIR wrap.
    // Spawn a task per invitee so a slow recipient doesn't block the
    // others. The outer command returns the call_id immediately.
    for invitee in participant_pubkeys.iter().cloned() {
        let task_state = state.inner().clone();
        let task_app = app.clone();
        let task_call_id = call_id.clone();
        let task_initiator = initiator_pubkey.clone();
        let task_my_pub = my_pub.to_vec();
        let task_participants = all_participants.clone();
        let task_call_key = call_key;
        // Re-derive my_secret per-task by re-fetching from the state
        // map (the StaticSecret was moved into the lock above; we
        // cloneable-wrap by re-fetching via a small reference).
        state
            .background_handles
            .lock()
            .push(tauri::async_runtime::spawn(async move {
                // Convert the invitee's Ed25519 identity pubkey to its
                // X25519 equivalent — same conversion DM uses to wrap
                // the conversation MEK per recipient
                // (services/dm/create.rs).
                let invitee_ed_bytes = match hex::decode(&invitee) {
                    Ok(b) if b.len() == 32 => {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&b);
                        arr
                    }
                    _ => {
                        tracing::warn!(peer = %invitee,
                            "invalid Ed25519 pubkey hex; skipping invitee");
                        return;
                    }
                };
                let invitee_x25519 = match rekindle_crypto::Identity::peer_ed25519_to_x25519(
                    &invitee_ed_bytes,
                ) {
                    Ok(p) => p.to_bytes().to_vec(),
                    Err(e) => {
                        tracing::warn!(peer = %invitee, error = %e,
                            "Ed25519→X25519 conversion failed");
                        return;
                    }
                };
                // Fetch our secret out of state (we left it there).
                let secret_clone = {
                    let calls = task_state.group_calls.lock();
                    calls
                        .get(&task_call_id)
                        .and_then(|c| c.our_x25519_secret.as_ref().map(|s| s.to_bytes()))
                };
                let secret_bytes = match secret_clone {
                    Some(b) => b,
                    None => return,
                };
                let secret_recovered = rekindle_calls::X25519StaticSecret::from(secret_bytes);
                let wrapped = match wrap_call_key(
                    &secret_recovered,
                    &invitee_x25519,
                    &task_call_id,
                    &invitee,
                    &task_call_key,
                ) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(error = %e, peer = %invitee,
                            "failed to wrap group call_key");
                        return;
                    }
                };
                let offer = MessagePayload::GroupCallOffer {
                    call_id: task_call_id.clone(),
                    offer_kind: kind.as_u8(),
                    initiator_pubkey: task_initiator.clone(),
                    initiator_x25519_pub: task_my_pub.clone(),
                    participants: task_participants.clone(),
                    wrapped_call_key: wrapped,
                    expires_at_ms,
                };
                match crate::services::message_service::send_to_peer_call(
                    &task_state,
                    &invitee,
                    &offer,
                )
                .await
                {
                    Ok(MessagePayload::GroupCallAccept {
                        call_id: reply_id,
                        acceptor_pubkey,
                    }) if reply_id == task_call_id => {
                        {
                            let mut calls = task_state.group_calls.lock();
                            if let Some(c) = calls.get_mut(&task_call_id) {
                                c.accepted.insert(acceptor_pubkey.clone());
                                if c.status == GroupCallStatus::Outgoing {
                                    c.status = GroupCallStatus::Active;
                                    let _ = task_app.emit(
                                        "chat-event",
                                        &ChatEvent::GroupCallConnected {
                                            call_id: task_call_id.clone(),
                                        },
                                    );
                                }
                            }
                        }
                        let _ = task_app.emit(
                            "chat-event",
                            &ChatEvent::GroupCallParticipantJoined {
                                call_id: task_call_id.clone(),
                                participant_pubkey: acceptor_pubkey,
                            },
                        );
                    }
                    Ok(MessagePayload::GroupCallDecline { reason, .. }) => {
                        let _ = task_app.emit(
                            "chat-event",
                            &ChatEvent::GroupCallParticipantLeft {
                                call_id: task_call_id.clone(),
                                participant_pubkey: invitee.clone(),
                                reason,
                            },
                        );
                    }
                    Ok(other) => {
                        tracing::debug!(reply = ?other,
                            "unexpected GroupCallOffer reply");
                    }
                    Err(e) => {
                        tracing::info!(error = %e, peer = %invitee,
                            "group call offer send failed");
                    }
                }
            }));
    }

    Ok(call_id)
}

/// Wave 12 W12.9 — accept an incoming group call. Mirrors
/// `accept_dm_call`: flips the local state to Active and signals the
/// app_call oneshot so the inbound handler returns `GroupCallAccept`.
#[tauri::command]
pub async fn accept_group_call(
    call_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    use crate::services::group_calls::GroupCallStatus;
    {
        let mut calls = state.group_calls.lock();
        let call = calls
            .get_mut(&call_id)
            .ok_or("no incoming group call with that id")?;
        if call.status != GroupCallStatus::Incoming {
            return Err("group call is not in Incoming state".into());
        }
        call.status = GroupCallStatus::Active;
    }
    let tx = crate::services::call_signaling::take_pending_response(&call_id)
        .ok_or("no pending accept handshake — group call already resolved")?;
    let _ = tx.send(crate::services::call_signaling::IncomingDecision::Accept);
    Ok(())
}

/// Wave 12 W12.9 — decline an incoming group call.
#[tauri::command]
pub async fn decline_group_call(
    call_id: String,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    state.group_calls.lock().remove(&call_id);
    let tx = crate::services::call_signaling::take_pending_response(&call_id)
        .ok_or("no pending decline handshake")?;
    let _ = tx.send(crate::services::call_signaling::IncomingDecision::Decline(
        reason.unwrap_or_default(),
    ));
    Ok(())
}

/// Wave 12 W12.9 — leave / end a group call. If we're the last
/// participant, this tears down the call entirely.
#[tauri::command]
pub async fn end_group_call(
    call_id: String,
    reason: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let removed = state.group_calls.lock().remove(&call_id).is_some();
    if removed {
        let _ = app.emit(
            "chat-event",
            &ChatEvent::GroupCallEnded {
                call_id,
                reason: reason.unwrap_or_default(),
            },
        );
    }
    Ok(())
}

/// Wave 12 W12.11 — fire a fire-and-forget emoji reaction at the peer.
/// Loss is tolerable: reactions are eye-candy, not state.
#[tauri::command]
pub async fn send_call_reaction(
    call_id: String,
    emoji: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    if emoji.is_empty() || emoji.len() > 32 {
        return Err("emoji must be 1-32 bytes".into());
    }
    let peer_pubkey = {
        let calls = state.active_calls.lock();
        let call = calls
            .get(&call_id)
            .ok_or_else(|| format!("no active call with id {call_id}"))?;
        call.peer_pubkey.clone()
    };
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::CallReaction {
        call_id,
        emoji,
        timestamp_ms: rekindle_utils::timestamp_ms(),
    };
    if let Err(e) = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_pubkey,
        &payload,
    )
    .await
    {
        return Err(format!("send_call_reaction: {e}"));
    }
    Ok(())
}

/// Wave 12 W12.12 — temp-mute a caller. Future incoming offers from
/// this peer auto-decline with reason "user is unavailable" until the
/// expiry passes. Cleared on app restart (in-memory only) so the user
/// can't accidentally outlive a temp-mute set in a previous session.
#[tauri::command]
pub async fn mute_caller_temp(
    peer_public_key: String,
    duration_ms: u64,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let expires_at = rekindle_utils::timestamp_ms().saturating_add(duration_ms);
    state
        .temp_call_muted
        .lock()
        .insert(peer_public_key, expires_at);
    Ok(())
}

/// Wave 12 W12.6 — fire-and-forget mid-call media-state update.
/// Frontend invokes this on each mute / camera / screen-share toggle so
/// the peer's UI can mount or unmount the corresponding tile.
///
/// Send is best-effort via `app_message`. Loss is tolerable: the next
/// keyframe arrival (or absence of one) is the authoritative truth on
/// whether the peer's camera is on. The state-ping merely accelerates
/// the UI transition.
#[tauri::command]
pub async fn send_call_media_state(
    call_id: String,
    audio: bool,
    video: bool,
    screen: bool,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let peer_pubkey = {
        let calls = state.active_calls.lock();
        let call = calls
            .get(&call_id)
            .ok_or_else(|| format!("no active call with id {call_id}"))?;
        call.peer_pubkey.clone()
    };
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::CallMediaState {
        call_id,
        audio,
        video,
        screen,
        timestamp_ms: rekindle_utils::timestamp_ms(),
    };
    if let Err(e) = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_pubkey,
        &payload,
    )
    .await
    {
        return Err(format!("send_call_media_state: {e}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn decline_dm_call(
    call_id: String,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let tx = {
        let mut calls = state.active_calls.lock();
        let _ = calls.remove(&call_id);
        crate::services::call_signaling::take_pending_response(&call_id)
    };
    let tx = tx.ok_or("no pending decline handshake — call already resolved")?;
    let _ = tx.send(crate::services::call_signaling::IncomingDecision::Decline(
        reason.unwrap_or_default(),
    ));
    Ok(())
}

#[tauri::command]
pub async fn get_missed_calls(
    pool: State<'_, DbPool>,
    state: State<'_, SharedState>,
) -> Result<Vec<MissedCallRow>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT call_id, peer_key, kind, expired_at FROM missed_calls \
             WHERE owner_key = ?1 ORDER BY expired_at DESC LIMIT 200",
        )?;
        let rows = stmt
            .query_map(params![owner_key], |row| {
                Ok(MissedCallRow {
                    call_id: row.get::<_, String>(0)?,
                    peer_key: row.get::<_, String>(1)?,
                    kind: u8::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                    expired_at: row.get::<_, i64>(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissedCallRow {
    pub call_id: String,
    pub peer_key: String,
    pub kind: u8,
    pub expired_at: i64,
}

fn generate_call_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

async fn finalize_outgoing_accept(
    app: &tauri::AppHandle,
    state: &SharedState,
    call_id: &str,
    peer_public_key: &str,
    acceptor_x25519_pub: &[u8],
) -> Result<(), String> {
    let secret = {
        let mut calls = state.active_calls.lock();
        let call = calls
            .get_mut(call_id)
            .ok_or("call vanished before accept finalization")?;
        call.status = CallStatus::Active;
        let secret = call
            .my_x25519_secret
            .take()
            .ok_or("missing local X25519 secret")?;
        if acceptor_x25519_pub.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(acceptor_x25519_pub);
            call.peer_x25519_pub = Some(arr);
        }
        secret
    };

    let key = derive_call_key(&secret, acceptor_x25519_pub, call_id)
        .map_err(|e| format!("derive call key: {e}"))?;
    {
        let mut calls = state.active_calls.lock();
        if let Some(call) = calls.get_mut(call_id) {
            call.call_key = Some(key);
        }
    }

    crate::services::voice::session::start_session(peer_public_key, None, app, state).await?;
    let _ = app.emit(
        "chat-event",
        &ChatEvent::CallConnected {
            call_id: call_id.to_string(),
        },
    );
    Ok(())
}

fn persist_missed_call(
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
    db_fire(pool, "persist missed call", move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO missed_calls (call_id, owner_key, peer_key, kind, expired_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![cid, owner_key, pk, kind_u8, expired],
        )?;
        Ok(())
    });
}

/// Inbound-side helper invoked from `message_service` when a `CallOffer`
/// arrives via `app_call`. Returns the `MessagePayload` that should be
/// shipped back as the inline reply (Accept or Decline).
pub async fn handle_incoming_offer(
    app: &tauri::AppHandle,
    state: &SharedState,
    pool: &DbPool,
    sender_hex: &str,
    call_id: &str,
    offer_kind: u8,
    initiator_pubkey: &str,
    initiator_x25519_pub: &[u8],
    expires_at_ms: u64,
) -> MessagePayload {
    let kind = CallKind::from_u8(offer_kind).unwrap_or(CallKind::Audio);
    let display_name = state_helpers::friend_display_name(state, sender_hex)
        .unwrap_or_else(|| short_pubkey(initiator_pubkey));

    if initiator_x25519_pub.len() != 32 {
        return MessagePayload::CallDecline {
            call_id: call_id.to_string(),
            reason: "invalid x25519 public key".into(),
        };
    }

    // Wave 12 W12.12 — temp-muted caller short-circuit. Auto-decline
    // without ringing or emitting events so the user isn't bothered by
    // a peer they've explicitly silenced for the next hour. Expired
    // entries are pruned lazily on lookup.
    {
        let now = rekindle_utils::timestamp_ms();
        let mut muted = state.temp_call_muted.lock();
        if let Some(&expires_at) = muted.get(sender_hex) {
            if now < expires_at {
                tracing::info!(
                    peer = %sender_hex,
                    call_id = %call_id,
                    "auto-declining: caller is temp-muted"
                );
                return MessagePayload::CallDecline {
                    call_id: call_id.to_string(),
                    reason: "user is unavailable".into(),
                };
            }
            // Expired — clean up.
            muted.remove(sender_hex);
        }
    }
    let mut peer_arr = [0u8; 32];
    peer_arr.copy_from_slice(initiator_x25519_pub);

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

    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::services::call_signaling::insert_pending_response(call_id, tx);

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
    // Wave 12 W12.3 — sibling notification-event so a CLI / TUI frontend
    // can hook its own ringer (terminal bell, libnotify) without
    // implementing call-protocol logic. The Tauri GUI frontend uses
    // it to fire an OS-level notification and to drive the synthesized
    // ringtone (chat-event drives the in-app modal).
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

    // Wait for accept/decline or fall through to timeout. We use a
    // wall-clock deadline rather than `now + 30_000` to honour the
    // `expires_at_ms` chosen by the initiator.
    let now = rekindle_utils::timestamp_ms();
    let remaining = expires_at_ms.saturating_sub(now);
    let timeout = Duration::from_millis(remaining.max(1));

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(crate::services::call_signaling::IncomingDecision::Accept)) => {
            // Generate our X25519, derive the shared key, store it,
            // start the voice transport, then ship CallAccept.
            let (secret, our_pub) = fresh_keypair();
            let key = match derive_call_key(&secret, initiator_x25519_pub, call_id) {
                Ok(k) => k,
                Err(e) => {
                    crate::services::call_signaling::take_pending_response(call_id);
                    state.active_calls.lock().remove(call_id);
                    return MessagePayload::CallDecline {
                        call_id: call_id.to_string(),
                        reason: format!("derive failed: {e}"),
                    };
                }
            };
            {
                let mut calls = state.active_calls.lock();
                if let Some(call) = calls.get_mut(call_id) {
                    call.my_x25519_secret = Some(secret);
                    call.call_key = Some(key);
                    call.status = CallStatus::Active;
                }
            }
            if let Err(e) =
                crate::services::voice::session::start_session(sender_hex, None, app, state).await
            {
                tracing::warn!(error = %e, call = %call_id, "failed to start voice session for accepted call");
            }
            let _ = app.emit(
                "chat-event",
                &ChatEvent::CallConnected {
                    call_id: call_id.to_string(),
                },
            );
            MessagePayload::CallAccept {
                call_id: call_id.to_string(),
                acceptor_x25519_pub: our_pub.to_vec(),
            }
        }
        Ok(Ok(crate::services::call_signaling::IncomingDecision::Decline(reason))) => {
            state.active_calls.lock().remove(call_id);
            MessagePayload::CallDecline {
                call_id: call_id.to_string(),
                reason,
            }
        }
        Ok(Err(_)) | Err(_) => {
            // Sender dropped (oneshot closed) or timer fired. Either
            // way: ring expired.
            crate::services::call_signaling::take_pending_response(call_id);
            state.active_calls.lock().remove(call_id);
            persist_missed_call(pool, state, call_id, sender_hex, kind, expires_at_ms);
            let _ = app.emit(
                "chat-event",
                &ChatEvent::CallMissed {
                    call_id: call_id.to_string(),
                    from: sender_hex.to_string(),
                },
            );
            MessagePayload::CallDecline {
                call_id: call_id.to_string(),
                reason: "ring timeout".into(),
            }
        }
    }
}

fn short_pubkey(pk: &str) -> String {
    if pk.len() <= 12 {
        pk.to_string()
    } else {
        format!("{}…{}", &pk[..6], &pk[pk.len() - 4..])
    }
}
