use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::voice::{VoiceControls, VoiceJoined, VoiceLeft};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::services::voice_runtime::{
    build_voice_signaling_deps, list_audio_devices_inner, persist_audio_device_prefs,
    server_mute_member_inner, AudioDevices,
};
use crate::state::SharedState;

/// Join a voice channel — initialize the voice engine and emit join event.
///
/// For community voice channels, pass `community_id` so the transport uses
/// gossip-based peer discovery (VoiceJoin/VoiceLeave) instead of single-peer lookup.
#[tauri::command]
pub async fn join_voice_channel(
    channel_id: String,
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<VoiceJoined>, CommandError> {
    crate::host_call::call(
        state.inner(),
        IpcRequest::VoiceJoin {
            community: community_id,
            channel: channel_id,
            muted: false,
            deafened: false,
        },
    )
    .await
}

/// Leave the current voice channel.
#[tauri::command]
pub async fn leave_voice(state: State<'_, SharedState>) -> Result<Done<VoiceLeft>, CommandError> {
    crate::host_call::call(state.inner(), IpcRequest::VoiceLeave).await
}

/// Set microphone mute state.
///
/// Muting sets a flag — the send loop checks it and skips encoding.
/// The capture stream stays alive to avoid device re-initialization latency.
#[tauri::command]
pub async fn set_mute(
    muted: bool,
    state: State<'_, SharedState>,
) -> Result<Done<VoiceControls>, CommandError> {
    crate::host_call::call(state.inner(), IpcRequest::VoiceSetMute { muted }).await
}

/// Set deafen state (mute all audio output).
///
/// Deafening sets a flag — the receive loop sends silence to playback.
/// The playback stream stays alive to avoid device re-initialization latency.
#[tauri::command]
pub async fn set_deafen(
    deafened: bool,
    state: State<'_, SharedState>,
) -> Result<Done<VoiceControls>, CommandError> {
    crate::host_call::call(state.inner(), IpcRequest::VoiceSetDeafen { deafened }).await
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
    state: State<'_, SharedState>,
) -> Result<(), String> {
    persist_audio_device_prefs(
        state.inner(),
        input_device.as_deref(),
        output_device.as_deref(),
    )
    .await?;
    state
        .host
        .get()
        .await
        .reopen_voice_devices(input_device, output_device)
        .await
}

/// Save the input-channel choice for input `device` (0-based channel
/// indices; empty means every channel, averaged) and, mid-call, reopen
/// capture on it (plan C7.24b, Mumble's input channel mask).
#[tauri::command]
pub async fn set_input_channels(
    device: String,
    channels: Vec<u16>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::services::voice_runtime::persist_input_channels(state.inner(), &device, channels)
        .await?;
    let prefs = crate::services::preferences::current(state.inner());
    state
        .host
        .get()
        .await
        .reopen_voice_devices(prefs.input_device, prefs.output_device)
        .await
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
    state: State<'_, SharedState>,
) -> Result<(), String> {
    state.db.current()?;
    state
        .host
        .get()
        .await
        .set_voice_mode(&mode, host_pseudonym)
        .await
}

/// Server-mute a member in a community voice channel (moderator action).
#[tauri::command]
pub async fn server_mute_member(
    community_id: String,
    channel_id: Option<String>,
    target_pseudonym: String,
    muted: bool,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    server_mute_member_inner(
        &community_id,
        channel_id,
        &target_pseudonym,
        muted,
        state.inner(),
    )
}

#[tauri::command]
pub async fn server_deafen_member(
    community_id: String,
    channel_id: Option<String>,
    target_pseudonym: String,
    deafened: bool,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let channel_id = crate::services::voice_runtime::voice_channel_or(
        channel_id,
        crate::services::voice_runtime::DEAFEN_NEEDS_VOICE,
    )?;
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_types::permissions::DEAFEN_MEMBERS,
    )?;
    let deps = build_voice_signaling_deps(state.inner())?;
    rekindle_voice::signaling::server_deafen_member(
        &deps,
        &community_id,
        &channel_id,
        &target_pseudonym,
        deafened,
    );
    Ok(())
}

#[tauri::command]
pub async fn request_to_speak(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_types::permissions::REQUEST_TO_SPEAK,
    )?;
    let deps = build_voice_signaling_deps(state.inner())?;
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
        rekindle_types::permissions::VIEW_CHANNELS,
    )?;
    crate::services::community::list_hand_raises(state.inner(), &community_id, &channel_id).await
}

#[tauri::command]
pub async fn respond_to_speak_request(
    community_id: String,
    channel_id: String,
    requester_pseudonym: String,
    granted: bool,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_types::permissions::MANAGE_MESSAGES,
    )?;
    let deps = build_voice_signaling_deps(state.inner())?;
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
