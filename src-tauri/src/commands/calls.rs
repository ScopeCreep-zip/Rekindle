//! Wave 13 / Architecture §10.10 — direct call signaling (1:1 + group).
//!
//! All five Tauri commands fire-and-forget envelopes via `app_message`,
//! mirroring the friend-add handshake. No Veilid `app_call` anywhere
//! in this file — the old RPC-with-30s-inline-reply pattern is gone.
//!
//! 1:1 commands:
//! * [`start_dm_call`]    — caller: insert CallState=Outgoing, fire
//!   CallInvite, return `call_id` immediately. UI shows "Calling…"
//!   until ChatEvent::{CallConnected,CallDeclined,CallTimedOut} fires.
//! * [`accept_dm_call`]   — receiver: derive call_key, start voice
//!   session, fire CallAccept envelope, emit CallConnected locally.
//! * [`decline_dm_call`]  — receiver: drop CallState, fire CallDecline
//!   envelope.
//! * [`end_dm_call`]      — either party: fire CallEnd envelope, drop
//!   CallState, emit CallEnded locally. Works in any state.
//! * [`send_call_media_state`] — Wave 12 W12.6 mid-call state ping.
//! * [`send_call_reaction`]    — Wave 12 W12.11 emoji reaction.
//! * [`mute_caller_temp`]      — Wave 12 W12.12 silence-this-caller.
//!
//! Group commands flip the same way; per-recipient X25519 wrap stays
//! intact (W12.9). See [`start_group_call`].

use rand::RngCore;
use rusqlite::params;
use tauri::{Emitter, State};

use rekindle_calls::{derive_call_key, fresh_keypair, CallKind, CallState, CallStatus};
use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

/// Architecture §10.10 — 30 s ring before the call is logged as missed.
const RING_DURATION_MS: u64 = 30_000;

// ── 1:1 direct calls ────────────────────────────────────────────────────

/// W13.3 — fire-and-forget caller. Generates call_id, inserts
/// CallState=Outgoing, fires CallInvite via app_message, returns
/// immediately. Replies arrive asynchronously as separate inbound
/// envelopes (CallAccept / CallDecline / CallRinging) and as the local
/// 30 s ring timeout.
#[tauri::command]
pub async fn start_dm_call(
    peer_public_key: String,
    video: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let call_id = generate_call_id();
    let kind = if video { CallKind::Video } else { CallKind::Audio };
    let (my_secret, my_pub) = fresh_keypair();
    let initiator_pubkey = state_helpers::current_identity(state.inner())
        .map_err(|_| "identity not set".to_string())?
        .public_key;
    let expires_at_ms = rekindle_utils::timestamp_ms() + RING_DURATION_MS;

    // Insert state BEFORE sending — a fast peer ack must find the entry.
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

    // Schedule the 30 s ring timer (services/calls/ring_timer.rs).
    crate::services::calls::ring_timer::spawn_dialing_timeout(
        state.inner().clone(),
        pool.inner().clone(),
        app.clone(),
        call_id.clone(),
        peer_public_key.clone(),
        kind,
        expires_at_ms,
    );

    // Fire the invite. Mirrors send_friend_request shape (fire-and-forget).
    let invite = MessagePayload::CallInvite {
        call_id: call_id.clone(),
        offer_kind: kind.as_u8(),
        initiator_pubkey,
        initiator_x25519_pub: my_pub.to_vec(),
        expires_at_ms,
    };
    if let Err(e) = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_public_key,
        &invite,
    )
    .await
    {
        // Couldn't ship the invite at all — drop the call entry, return error.
        state.active_calls.lock().remove(&call_id);
        return Err(format!("call invite send failed: {e}"));
    }

    Ok(call_id)
}

/// W13.5 — receiver accepts. Order matters:
/// (1) generate X25519, derive call_key,
/// (2) start voice session locally,
/// (3) fire CallAccept envelope (so by the time the caller acts on it
///     our voice is already up to receive their first packets),
/// (4) emit local CallConnected.
#[tauri::command]
pub async fn accept_dm_call(
    call_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Look up + validate.
    let (peer_pubkey, peer_x25519, my_pub) = {
        let mut calls = state.active_calls.lock();
        let call = calls
            .get_mut(&call_id)
            .ok_or("no incoming call with that id")?;
        if !matches!(call.status, CallStatus::Incoming) {
            return Err("call is not in Incoming state".into());
        }
        let peer_x25519 = call
            .peer_x25519_pub
            .ok_or("invite was malformed (missing peer x25519)")?;
        let (my_secret, my_pub) = fresh_keypair();
        let call_key = derive_call_key(&my_secret, &peer_x25519, &call_id)
            .map_err(|e| format!("derive call key: {e}"))?;
        call.my_x25519_secret = Some(my_secret);
        call.call_key = Some(call_key);
        call.status = CallStatus::Connecting;
        (call.peer_pubkey.clone(), peer_x25519, my_pub)
    };
    let _ = peer_x25519; // value already in CallState; kept here only to enforce
                        // that the read happened before voice session start

    // Start voice session BEFORE sending the accept.
    if let Err(e) = crate::services::voice::session::start_session(
        &peer_pubkey,
        None,
        &app,
        state.inner(),
    )
    .await
    {
        state.active_calls.lock().remove(&call_id);
        // Tell the caller we couldn't accept after all.
        let _ = crate::services::message_service::send_to_peer_raw(
            state.inner(),
            pool.inner(),
            &peer_pubkey,
            &MessagePayload::CallDecline {
                call_id: call_id.clone(),
                reason: format!("voice session failed: {e}"),
            },
        )
        .await;
        return Err(format!("voice session failed: {e}"));
    }

    // Voice up. Fire the accept envelope.
    let accept = MessagePayload::CallAccept {
        call_id: call_id.clone(),
        acceptor_x25519_pub: my_pub.to_vec(),
    };
    if let Err(e) = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_pubkey,
        &accept,
    )
    .await
    {
        // We're locally in-call but the peer never gets the accept.
        // Caller will time out their dialing; our local state will stick
        // around until our own incoming-timeout fires. Surface as warning,
        // don't unwind — the user clicked Accept and our voice is up; if
        // anything reaches them through any path, audio works.
        tracing::warn!(error = %e, call = %call_id,
            "CallAccept send failed; caller may time out");
    }

    // Local UI transitions to Active.
    {
        let mut calls = state.active_calls.lock();
        if let Some(c) = calls.get_mut(&call_id) {
            c.status = CallStatus::Active;
        }
    }
    let _ = app.emit("chat-event", &ChatEvent::CallConnected { call_id });
    Ok(())
}

/// W13.7 — receiver declines. Drop CallState, fire CallDecline.
#[tauri::command]
pub async fn decline_dm_call(
    call_id: String,
    reason: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let peer_pubkey = state
        .active_calls
        .lock()
        .remove(&call_id)
        .map(|c| c.peer_pubkey.clone())
        .ok_or("no incoming call with that id")?;
    let _ = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_pubkey,
        &MessagePayload::CallDecline {
            call_id,
            reason: reason.unwrap_or_default(),
        },
    )
    .await;
    Ok(())
}

/// Hangup (or cancel-while-dialing). Removes CallState, fires CallEnd
/// to peer, emits CallEnded locally. Works in any state — Outgoing /
/// Incoming / Connecting / Active — for cancel-while-ringing,
/// hangup-mid-call, and decline-after-accept races.
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
        call.peer_pubkey.clone()
    };
    let reason_str = reason.unwrap_or_default();
    let payload = MessagePayload::CallEnd {
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
        tracing::info!(call_id = %call_id, peer = %peer_pubkey, error = %e,
            "CallEnd send failed; their state will GC on their own timeout");
    }
    let _ = app.emit(
        "chat-event",
        &ChatEvent::CallEnded {
            call_id,
            reason: reason_str,
        },
    );
    Ok(())
}

// ── Mid-call state pings + reactions + temp-mute (W12.6 / W12.11 / W12.12) ──

/// W12.6 — mid-call media state change. Fire-and-forget app_message.
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
    let payload = MessagePayload::CallMediaState {
        call_id,
        audio,
        video,
        screen,
        timestamp_ms: rekindle_utils::timestamp_ms(),
    };
    crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_pubkey,
        &payload,
    )
    .await
    .map_err(|e| format!("send_call_media_state: {e}"))
}

/// W12.11 — fire-and-forget emoji reaction.
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
    let payload = MessagePayload::CallReaction {
        call_id,
        emoji,
        timestamp_ms: rekindle_utils::timestamp_ms(),
    };
    crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &peer_pubkey,
        &payload,
    )
    .await
    .map_err(|e| format!("send_call_reaction: {e}"))
}

/// W12.12 — temp-mute a caller. Future invites from this peer
/// auto-decline silently until the expiry passes. Cleared on app
/// restart (in-memory only) so the user never inherits a temp-mute
/// from a previous session.
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

// ── Group calls (W12.9 + W13.13) ────────────────────────────────────────

/// W13.13 — start a group call. Generates call_key, inserts
/// GroupCallState=Outgoing, fires per-recipient `GroupCallOffer`
/// envelopes via app_message (each carrying that invitee's wrapped
/// call_key). Returns the call_id immediately; replies arrive
/// asynchronously as ChatEvent::GroupCall* events.
#[tauri::command]
pub async fn start_group_call(
    participant_pubkeys: Vec<String>,
    video: bool,
    _app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    use crate::services::group_calls::{GroupCallState, GroupCallStatus};
    use rekindle_calls::group::{generate_call_key, wrap_call_key};

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

    let mut all_participants = vec![initiator_pubkey.clone()];
    for pk in &participant_pubkeys {
        if !all_participants.contains(pk) {
            all_participants.push(pk.clone());
        }
    }

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

    // Fire one CallInvite-shaped envelope per invitee, each carrying
    // their per-recipient wrapped call_key. Fire-and-forget, in
    // parallel via spawned tasks so a slow invitee doesn't block.
    for invitee in participant_pubkeys.iter().cloned() {
        let task_state = state.inner().clone();
        let task_pool = pool.inner().clone();
        let task_call_id = call_id.clone();
        let task_initiator = initiator_pubkey.clone();
        let task_my_pub = my_pub.to_vec();
        let task_participants = all_participants.clone();
        let task_call_key = call_key;
        let handle = tauri::async_runtime::spawn(async move {
            // Convert invitee's Ed25519 pubkey → X25519 (same path DM uses).
            let invitee_ed_bytes = match hex::decode(&invitee) {
                Ok(b) if b.len() == 32 => {
                    let mut a = [0u8; 32];
                    a.copy_from_slice(&b);
                    a
                }
                _ => {
                    tracing::warn!(peer = %invitee, "invalid Ed25519 pubkey hex; skipping");
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
            // Recover our secret out of state for the wrap.
            let secret_bytes = {
                let calls = task_state.group_calls.lock();
                calls
                    .get(&task_call_id)
                    .and_then(|c| c.our_x25519_secret.as_ref().map(|s| s.to_bytes()))
            };
            let Some(secret_bytes) = secret_bytes else {
                return;
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
            // W13.13 — fire-and-forget via app_message (was app_call).
            if let Err(e) = crate::services::message_service::send_to_peer_raw(
                &task_state,
                &task_pool,
                &invitee,
                &offer,
            )
            .await
            {
                tracing::info!(peer = %invitee, error = %e,
                    "GroupCallOffer send failed");
            }
        });
        state.background_handles.lock().push(handle);
    }

    Ok(call_id)
}

/// W13.13 — receiver accepts a group call. Fires GroupCallAccept
/// envelope to the initiator (and to other already-accepted peers via
/// gossip ParticipantJoined — handled in services::group_calls).
#[tauri::command]
pub async fn accept_group_call(
    call_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    use crate::services::group_calls::GroupCallStatus;
    let (initiator_pubkey, our_ed_pubkey) = {
        let mut calls = state.group_calls.lock();
        let call = calls
            .get_mut(&call_id)
            .ok_or("no incoming group call with that id")?;
        if call.status != GroupCallStatus::Incoming {
            return Err("group call is not in Incoming state".into());
        }
        call.status = GroupCallStatus::Active;
        (call.initiator_pubkey.clone(), state_helpers::current_identity(&state)
            .map(|i| i.public_key)
            .unwrap_or_default())
    };

    // TODO: voice session setup for group calls — W14 follow-up.
    // For now we mark Active and fire the accept; the voice topology
    // election (mesh ≤4 / SFU >4) layers on later.

    let payload = MessagePayload::GroupCallAccept {
        call_id: call_id.clone(),
        acceptor_pubkey: our_ed_pubkey,
    };
    let _ = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &initiator_pubkey,
        &payload,
    )
    .await;

    let _ = app.emit(
        "chat-event",
        &ChatEvent::GroupCallConnected { call_id },
    );
    Ok(())
}

/// W13.13 — receiver declines a group call.
#[tauri::command]
pub async fn decline_group_call(
    call_id: String,
    reason: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let initiator_pubkey = state
        .group_calls
        .lock()
        .remove(&call_id)
        .map(|c| c.initiator_pubkey.clone())
        .ok_or("no incoming group call with that id")?;
    let payload = MessagePayload::GroupCallDecline {
        call_id,
        reason: reason.unwrap_or_default(),
    };
    let _ = crate::services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &initiator_pubkey,
        &payload,
    )
    .await;
    Ok(())
}

/// Leave / end a group call.
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

// ── Missed-call query ──────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissedCallRow {
    pub call_id: String,
    pub peer_key: String,
    pub kind: u8,
    pub expired_at: i64,
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

// ── Internal helpers ───────────────────────────────────────────────────

fn generate_call_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
