use tauri::{Emitter, State};
use tauri_plugin_store::StoreExt;

use crate::channels::VoiceEvent;
use crate::state::SharedState;
use crate::state_helpers;

// Re-export so auth.rs, lib.rs, etc. can `use crate::commands::voice::{shutdown_voice, VoiceShutdownOpts}`
pub(crate) use crate::services::voice::shutdown::{shutdown_voice, VoiceShutdownOpts};

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
    // Check CONNECT permission for community voice channels
    if let Some(ref cid) = community_id {
        crate::commands::community::require_permission(
            state.inner(),
            cid,
            rekindle_protocol::dht::community::permissions_v2::Permissions::CONNECT,
        )?;
    }

    crate::services::voice::session::start_session(
        &channel_id,
        community_id.as_deref(),
        &app,
        state.inner(),
    )
    .await?;

    if let Some(ref cid) = community_id {
        maybe_apply_stage_audience_gate(state.inner(), cid, &channel_id);
        log_voice_join_event(&state, cid, &channel_id, &app);
    }

    // Broadcast VoiceJoin via gossip so other community members add us as a peer
    if let Some(ref cid) = community_id {
        let route_blob = crate::state_helpers::our_route_blob(&state).unwrap_or_default();
        let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::VoiceJoin {
                channel_id: channel_id.clone(),
                route_blob,
            },
        );
        let _ = crate::services::community::send_to_mesh(state.inner(), cid, &envelope);
    }

    Ok(())
}

/// Leave the current voice channel.
#[tauri::command]
pub async fn leave_voice(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let public_key = state_helpers::owner_key_or_default(state.inner());

    // Broadcast VoiceLeave via gossip before shutdown so other members
    // remove us from their transport immediately (not waiting for 5s stale timeout).
    let (channel_id, community_id) = {
        let ve = state.voice_engine.lock();
        ve.as_ref()
            .map(|h| (h.channel_id.clone(), h.community_id.clone()))
            .unwrap_or_default()
    };
    if let Some(ref cid) = community_id {
        log_voice_leave_event(&state, cid, &channel_id, &app);
        let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::VoiceLeave {
                channel_id: channel_id.clone(),
            },
        );
        let _ = crate::services::community::send_to_mesh(state.inner(), cid, &envelope);
    }

    shutdown_voice(&state, &VoiceShutdownOpts::FULL).await;

    // Emit leave event
    let event = VoiceEvent::UserLeft { public_key };
    let _ = app.emit("voice-event", &event);

    Ok(())
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
    if let Some(ref mut handle) = *state.voice_engine.lock() {
        handle.engine.set_muted(muted);
        handle
            .muted_flag
            .store(muted, std::sync::atomic::Ordering::Relaxed);
    }

    let public_key = state_helpers::owner_key_or_default(state.inner());

    let event = VoiceEvent::UserMuted { public_key, muted };
    let _ = app.emit("voice-event", &event);

    Ok(())
}

/// Set deafen state (mute all audio output).
///
/// Deafening sets a flag — the receive loop sends silence to playback.
/// The playback stream stays alive to avoid device re-initialization latency.
#[tauri::command]
pub async fn set_deafen(deafened: bool, state: State<'_, SharedState>) -> Result<(), String> {
    if let Some(ref mut handle) = *state.voice_engine.lock() {
        handle.engine.set_deafened(deafened);
        handle
            .deafened_flag
            .store(deafened, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

/// List available audio input and output devices.
#[tauri::command]
pub fn list_audio_devices() -> AudioDevices {
    let devices = rekindle_voice::capture::enumerate_audio_devices();

    AudioDevices {
        input_devices: devices
            .input_devices
            .into_iter()
            .map(|(name, is_default)| AudioDeviceInfo {
                id: name.clone(),
                name,
                is_default,
            })
            .collect(),
        output_devices: devices
            .output_devices
            .into_iter()
            .map(|(name, is_default)| AudioDeviceInfo {
                id: name.clone(),
                name,
                is_default,
            })
            .collect(),
    }
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
    // Persist to preferences store
    let store = app.store("preferences.json").map_err(|e| e.to_string())?;
    let mut prefs: crate::commands::settings::Preferences = store
        .get("preferences")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    prefs.input_device.clone_from(&input_device);
    prefs.output_device.clone_from(&output_device);
    let val = serde_json::to_value(&prefs).map_err(|e| e.to_string())?;
    store.set("preferences", val);
    store.save().map_err(|e| e.to_string())?;

    // If a voice call is active, hot-swap devices
    let is_active = state.voice_engine.lock().is_some();
    if is_active {
        tracing::info!(
            ?input_device,
            ?output_device,
            "hot-swapping audio devices mid-call"
        );

        // Shut down current loops
        shutdown_voice(&state, &VoiceShutdownOpts::KEEP_ENGINE).await;

        // Stop current capture/playback and update device config
        {
            let mut ve = state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                handle.engine.stop_capture();
                handle.engine.stop_playback();
                handle
                    .engine
                    .set_devices(input_device.clone(), output_device.clone());
            }
        }

        // Restart with new devices
        crate::services::voice::session::restart_loops(&state, &app).await?;
    } else {
        tracing::info!(
            ?input_device,
            ?output_device,
            "audio device preferences updated (takes effect on next call)"
        );
    }

    Ok(())
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
    // Stop any existing MCU loop
    crate::services::voice::session::stop_mcu_loop(state.inner()).await;

    // Set mode on the shared transport
    // Clone Arc out of parking_lot guard before .await
    let maybe_transport = {
        let ve = state.voice_engine.lock();
        ve.as_ref().map(|handle| handle.transport.clone())
    };
    if let Some(transport) = maybe_transport {
        let mode_enum = if mode == "mcu" {
            rekindle_voice::VoiceMode::Mcu {
                host_pseudonym: host_pseudonym.clone().unwrap_or_default(),
            }
        } else {
            rekindle_voice::VoiceMode::Mesh
        };
        transport.lock().await.set_mode(mode_enum);
    }

    if mode == "mcu" {
        if let Some(ref host) = host_pseudonym {
            let my_pseudonym = state_helpers::owner_key_or_default(state.inner());
            if *host == my_pseudonym {
                // We're the voice host — start MCU loop using shared transport
                crate::services::voice::session::start_mcu_loop(state.inner())?;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevices {
    pub input_devices: Vec<AudioDeviceInfo>,
    pub output_devices: Vec<AudioDeviceInfo>,
}

/// Server-mute a member in a community voice channel (moderator action).
#[tauri::command]
pub async fn server_mute_member(
    community_id: String,
    channel_id: String,
    target_pseudonym: String,
    muted: bool,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::MUTE_MEMBERS,
    )?;

    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::VoiceMute {
            channel_id,
            target_pseudonym,
            muted,
        },
    );
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)?;
    Ok(())
}

/// Server-deafen a member in a community voice channel (moderator action).
#[tauri::command]
pub async fn server_deafen_member(
    community_id: String,
    channel_id: String,
    target_pseudonym: String,
    deafened: bool,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::DEAFEN_MEMBERS,
    )?;

    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::VoiceDeafen {
            channel_id,
            target_pseudonym,
            deafened,
        },
    );
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)?;
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
        rekindle_protocol::dht::community::permissions_v2::Permissions::REQUEST_TO_SPEAK,
    )?;
    let requester_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|community| community.my_pseudonym_key.clone())
            .ok_or("no pseudonym key for this community")?
    };
    crate::services::community::persist_hand_raise(
        state.inner(),
        &community_id,
        &channel_id,
        true,
    )
    .await?;
    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::SpeakRequest {
            channel_id,
            requester_pseudonym,
            lamport,
        },
    );
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)?;
    Ok(())
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
    let moderator_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|community| community.my_pseudonym_key.clone())
            .ok_or("no pseudonym key for this community")?
    };

    if granted {
        crate::services::community::rotate_voice_mek_for_membership(
            &app_handle,
            state.inner(),
            &community_id,
            &channel_id,
            &requester_pseudonym,
            true,
        )
        .await?;
    }

    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::SpeakResponse {
            channel_id: channel_id.clone(),
            requester_pseudonym: requester_pseudonym.clone(),
            granted,
            moderator_pseudonym: moderator_pseudonym.clone(),
            lamport,
        },
    );
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)?;

    if granted {
        let speakers = {
            let communities = state.communities.read();
            let channel = communities
                .get(&community_id)
                .and_then(|community| community.channels.iter().find(|channel| channel.id == channel_id))
                .ok_or("channel not found")?;
            let mut speakers = channel.stage_speakers.clone();
            if !speakers.contains(&requester_pseudonym) {
                speakers.push(requester_pseudonym.clone());
            }
            speakers
        };
        let stage_update = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::StageUpdate {
                channel_id,
                topic: None,
                speakers,
                moderator_pseudonym,
                lamport: lamport.saturating_add(1),
            },
        );
        crate::services::community::send_to_mesh(state.inner(), &community_id, &stage_update)?;
    }

    Ok(())
}

fn log_voice_join_event(
    state: &State<'_, SharedState>,
    community_id: &str,
    channel_id: &str,
    app: &tauri::AppHandle,
) {
    use tauri::Manager as _;
    let owner = state_helpers::owner_key_or_default(state.inner());
    let pseudo = my_pseudonym_for(state.inner(), community_id);
    let pool: tauri::State<'_, crate::db::DbPool> = app.state();
    crate::services::community::analytics::log_voice_join(
        pool.inner(),
        &owner,
        community_id,
        channel_id,
        &pseudo,
    );
}

fn log_voice_leave_event(
    state: &State<'_, SharedState>,
    community_id: &str,
    channel_id: &str,
    app: &tauri::AppHandle,
) {
    use tauri::Manager as _;
    let owner = state_helpers::owner_key_or_default(state.inner());
    let pseudo = my_pseudonym_for(state.inner(), community_id);
    let pool: tauri::State<'_, crate::db::DbPool> = app.state();
    crate::services::community::analytics::log_voice_leave(
        pool.inner(),
        &owner,
        community_id,
        channel_id,
        &pseudo,
    );
}

fn my_pseudonym_for(state: &SharedState, community_id: &str) -> String {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|c| c.my_pseudonym_key.clone())
        .unwrap_or_default()
}

fn maybe_apply_stage_audience_gate(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) {
    let (is_stage, my_pseudonym, speakers) = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return;
        };
        let Some(channel) = community.channels.iter().find(|channel| channel.id == channel_id) else {
            return;
        };
        (
            matches!(channel.channel_type, crate::state::ChannelType::Stage),
            community.my_pseudonym_key.clone().unwrap_or_default(),
            channel.stage_speakers.clone(),
        )
    };

    if !is_stage || speakers.contains(&my_pseudonym) {
        return;
    }

    let mut ve = state.voice_engine.lock();
    if let Some(ref mut handle) = *ve {
        handle.engine.set_muted(true);
        handle
            .muted_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
