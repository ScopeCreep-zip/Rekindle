use tauri::State;

use crate::services::voice_runtime::{
    build_voice_session_deps, build_voice_signaling_deps, join_voice_channel_inner,
    list_audio_devices_inner, persist_audio_device_prefs, server_mute_member_inner,
    AudioDevices,
};
use crate::state::SharedState;

// Re-export so auth.rs, lib.rs, etc. can use them via the legacy
// `commands::voice::{shutdown_voice, VoiceShutdownOpts}` import path.
// The actual bodies live in `services::voice_adapter` (shutdown_voice
// free fn) + `rekindle_voice::VoiceShutdownOpts` (the opts type).
pub(crate) use crate::services::voice_adapter::shutdown_voice;
pub(crate) use rekindle_voice::VoiceShutdownOpts;

/// Join a voice channel — initialize the voice engine and emit join event.
///
/// For community voice channels, pass `community_id` so the transport uses
/// gossip-based peer discovery (VoiceJoin/VoiceLeave) instead of single-peer lookup.
#[tauri::command]
pub async fn join_voice_channel(
    channel_id: String,
    community_id: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    join_voice_channel_inner(&channel_id, community_id.as_deref(), &app, state.inner()).await
}

/// Leave the current voice channel.
#[tauri::command]
pub async fn leave_voice(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let deps = build_voice_session_deps(&app, state.inner())?;
    rekindle_voice::session::leave_voice(&deps)
        .await
        .map_err(|e| e.to_string())
}

/// Set microphone mute state.
///
/// Muting sets a flag — the send loop checks it and skips encoding.
/// The capture stream stays alive to avoid device re-initialization latency.
#[tauri::command]
pub async fn set_mute(
    muted: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let deps = build_voice_session_deps(&app, state.inner())?;
    rekindle_voice::session::set_local_mute(&deps, muted);
    Ok(())
}

/// Set deafen state (mute all audio output).
///
/// Deafening sets a flag — the receive loop sends silence to playback.
/// The playback stream stays alive to avoid device re-initialization latency.
#[tauri::command]
pub async fn set_deafen(
    deafened: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let deps = build_voice_session_deps(&app, state.inner())?;
    rekindle_voice::session::set_local_deafen(&deps, deafened);
    Ok(())
}

/// List available audio input and output devices.
#[tauri::command]
pub fn list_audio_devices() -> AudioDevices {
    list_audio_devices_inner()
}

/// Set the preferred audio devices for voice calls.
///
/// Persists the selection to preferences. If a voice call is active, performs
/// a hot-swap by restarting capture/playback with the new devices (~100ms interruption).
#[tauri::command]
pub async fn set_audio_devices(
    input_device: Option<String>,
    output_device: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    persist_audio_device_prefs(&app, input_device.as_deref(), output_device.as_deref())?;
    let deps = build_voice_session_deps(&app, state.inner())?;
    rekindle_voice::session::change_audio_devices(&deps, input_device, output_device)
        .await
        .map_err(|e| e.to_string())
}

/// Switch voice mode between mesh and MCU.
///
/// When switching to MCU mode with ourselves as host, starts the MCU mixing
/// loop. When switching away from MCU (or another host), stops any running
/// MCU loop.
#[tauri::command]
pub async fn set_voice_mode(
    mode: String,
    host_pseudonym: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let deps = build_voice_session_deps(&app, state.inner())?;
    rekindle_voice::session::set_voice_mode(&deps, &mode, host_pseudonym)
        .await
        .map_err(|e| e.to_string())
}

/// Server-mute a member in a community voice channel (moderator action).
#[tauri::command]
pub async fn server_mute_member(
    community_id: String,
    channel_id: String,
    target_pseudonym: String,
    muted: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    server_mute_member_inner(
        &community_id,
        &channel_id,
        &target_pseudonym,
        muted,
        &app,
        state.inner(),
    )
}

#[tauri::command]
pub async fn server_deafen_member(
    community_id: String,
    channel_id: String,
    target_pseudonym: String,
    deafened: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::DEAFEN_MEMBERS,
    )?;
    let deps = build_voice_signaling_deps(&app, state.inner())?;
    rekindle_voice::signaling::server_deafen_member(&deps, &community_id, &channel_id, &target_pseudonym, deafened);
    Ok(())
}

#[tauri::command]
pub async fn request_to_speak(
    community_id: String,
    channel_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::REQUEST_TO_SPEAK,
    )?;
    let deps = build_voice_signaling_deps(&app, state.inner())?;
    rekindle_voice::signaling::request_to_speak(&deps, &community_id, &channel_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_stage_hand_raises(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
) -> Result<Vec<String>, String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::VIEW_CHANNEL,
    )?;
    crate::services::community::list_hand_raises(state.inner(), &community_id, &channel_id).await
}

#[tauri::command]
pub async fn respond_to_speak_request(
    community_id: String,
    channel_id: String,
    requester_pseudonym: String,
    granted: bool,
    app_handle: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::MANAGE_MESSAGES,
    )?;
    let deps = build_voice_signaling_deps(&app_handle, state.inner())?;
    rekindle_voice::signaling::respond_to_speak_request(
        &deps,
        &community_id,
        &channel_id,
        &requester_pseudonym,
        granted,
    )
    .await
    .map_err(|e| e.to_string())
}

// log_voice_join_event / log_voice_leave_event / my_pseudonym_for
// helpers deleted in Phase 14.o — moved into
// `VoiceAdapter::log_voice_membership` (called by the crate's
// `leave_voice` / `join_voice_channel` orchestrators).


