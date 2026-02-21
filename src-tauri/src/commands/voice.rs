use tauri::{Emitter, State};
use tauri_plugin_store::StoreExt;

use crate::channels::VoiceEvent;
use crate::state::SharedState;
use crate::state_helpers;

// Re-export so auth.rs, lib.rs, etc. can `use crate::commands::voice::{shutdown_voice, VoiceShutdownOpts}`
pub(crate) use crate::services::voice::shutdown::{shutdown_voice, VoiceShutdownOpts};

/// Join a voice channel — initialize the voice engine and emit join event.
#[tauri::command]
pub async fn join_voice_channel(
    channel_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::services::voice::session::start_session(&channel_id, &app, state.inner())
}

/// Leave the current voice channel.
#[tauri::command]
pub async fn leave_voice(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let public_key = state_helpers::owner_key_or_default(state.inner());

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
pub fn list_audio_devices() -> Result<AudioDevices, String> {
    let devices = rekindle_voice::capture::enumerate_audio_devices()
        .map_err(|e| format!("failed to enumerate devices: {e}"))?;

    Ok(AudioDevices {
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
    })
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
        crate::services::voice::session::restart_loops(&state, &app)?;
    } else {
        tracing::info!(
            ?input_device,
            ?output_device,
            "audio device preferences updated (takes effect on next call)"
        );
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
