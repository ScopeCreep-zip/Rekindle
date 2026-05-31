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

use tauri::State;

use rekindle_calls::CallKind;

use crate::db::DbPool;
use crate::services::call_runtime::{
    get_missed_calls_inner, mute_caller_temp_inner, send_call_media_state_inner,
    send_call_reaction_inner,
};
use crate::state::SharedState;

pub use crate::services::call_runtime::MissedCallRow;

fn build_adapter(
    app: tauri::AppHandle,
    state: &State<'_, SharedState>,
    pool: &State<'_, DbPool>,
) -> std::sync::Arc<crate::services::calls_adapter::CallsAdapter> {
    crate::services::calls_adapter::CallsAdapter::new(
        state.inner().clone(),
        app,
        pool.inner().clone(),
    )
}

// `RING_DURATION_MS` + `generate_call_id` moved into rekindle-calls
// (`signaling::outbound::RING_DURATION_MS` + `CallSignalingDeps::fresh_call_id`).

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
    let kind = if video { CallKind::Video } else { CallKind::Audio };
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::start_dm_call(adapter.as_ref(), &peer_public_key, kind)
        .await
        .map_err(|e| e.to_string())
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
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::accept_dm_call(adapter.as_ref(), &call_id)
        .await
        .map_err(|e| e.to_string())
}

/// W13.7 — receiver declines. Drop CallState, fire CallDecline.
#[tauri::command]
pub async fn decline_dm_call(
    call_id: String,
    reason: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::decline_dm_call(adapter.as_ref(), &call_id, reason)
        .await
        .map_err(|e| e.to_string())
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
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::end_dm_call(adapter.as_ref(), &call_id, reason)
        .await
        .map_err(|e| e.to_string())
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
    send_call_media_state_inner(state.inner(), pool.inner(), call_id, audio, video, screen).await
}

/// W12.11 — fire-and-forget emoji reaction.
#[tauri::command]
pub async fn send_call_reaction(
    call_id: String,
    emoji: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    send_call_reaction_inner(state.inner(), pool.inner(), call_id, emoji).await
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
    mute_caller_temp_inner(state.inner(), peer_public_key, duration_ms);
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
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    if participant_pubkeys.is_empty() {
        return Err("group call requires at least one invitee".into());
    }
    let kind = if video { CallKind::Video } else { CallKind::Audio };
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::start_group_call(adapter.as_ref(), participant_pubkeys, kind)
        .await
        .map_err(|e| e.to_string())
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
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::accept_group_call(adapter.as_ref(), &call_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn decline_group_call(
    call_id: String,
    reason: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::decline_group_call(adapter.as_ref(), &call_id, reason)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn end_group_call(
    call_id: String,
    reason: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let adapter = build_adapter(app, &state, &pool);
    rekindle_calls::signaling::end_group_call(adapter.as_ref(), &call_id, reason)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_missed_calls(
    pool: State<'_, DbPool>,
    state: State<'_, SharedState>,
) -> Result<Vec<MissedCallRow>, String> {
    get_missed_calls_inner(state.inner(), pool.inner()).await
}

// `generate_call_id` deleted in Phase 14.n — moved into
// `CallSignalingDeps::fresh_call_id` (default trait method).
