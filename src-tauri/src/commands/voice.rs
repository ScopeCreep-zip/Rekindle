use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{Emitter, State};
use tauri_plugin_store::StoreExt;
use tokio::sync::{broadcast, mpsc};

use crate::channels::{NotificationEvent, VoiceEvent};
use crate::state::{SharedState, VoiceEngineHandle};
use crate::state_helpers;

/// Join a voice channel — initialize the voice engine and emit join event.
#[allow(clippy::too_many_lines)]
#[tauri::command]
pub async fn join_voice_channel(
    channel_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    // Check if already in a call
    {
        let ve = state.voice_engine.lock();
        if let Some(ref handle) = *ve {
            if handle.channel_id == channel_id {
                return Err("already in this voice channel".to_string());
            }
            return Err(format!("already in voice channel {}", handle.channel_id));
        }
    }

    let identity = state_helpers::current_identity(state.inner())?;

    // Load audio device preferences from persistent store
    let prefs: crate::commands::settings::Preferences = app
        .store("preferences.json")
        .ok()
        .and_then(|store| store.get("preferences"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    // Initialize voice engine
    let muted_flag = Arc::new(AtomicBool::new(false));
    let deafened_flag = Arc::new(AtomicBool::new(false));
    {
        let config = rekindle_voice::VoiceConfig {
            input_device: prefs.input_device,
            output_device: prefs.output_device,
            input_volume: prefs.input_volume,
            output_volume: prefs.output_volume,
            noise_suppression: prefs.noise_suppression,
            echo_cancellation: prefs.echo_cancellation,
            ..rekindle_voice::VoiceConfig::default()
        };
        let engine = rekindle_voice::VoiceEngine::new(config)
            .map_err(|e| format!("failed to create voice engine: {e}"))?;
        *state.voice_engine.lock() = Some(VoiceEngineHandle {
            engine,
            send_loop_shutdown: None,
            send_loop_handle: None,
            recv_loop_shutdown: None,
            recv_loop_handle: None,
            device_monitor_shutdown: None,
            device_monitor_handle: None,
            channel_id: channel_id.clone(),
            muted_flag: Arc::clone(&muted_flag),
            deafened_flag: Arc::clone(&deafened_flag),
        });
    }

    // Start audio capture and playback
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle
                .engine
                .start_capture()
                .map_err(|e| format!("failed to start capture: {e}"))?;
            handle
                .engine
                .start_playback()
                .map_err(|e| format!("failed to start playback: {e}"))?;
        }
    }

    // Create voice transport for this channel and attempt to connect.
    let mut transport = rekindle_voice::transport::VoiceTransport::new(channel_id.clone());

    // Try to look up a route blob for this channel
    let route_blob = state_helpers::cached_route_blob(state.inner(), &channel_id);

    // Clone API handle out before await
    let api = state_helpers::veilid_api(state.inner());

    if let (Some(blob), Some(api)) = (route_blob, api) {
        let sender_key = hex::decode(&identity.public_key).unwrap_or_default();
        if let Err(e) = transport.connect(api, &blob, sender_key) {
            tracing::warn!(error = %e, channel = %channel_id, "voice transport connect failed — audio only local");
        }
    }

    // Take capture_rx and playback_tx from the engine
    let (capture_rx, playback_tx) = {
        let mut ve = state.voice_engine.lock();
        let veh = ve.as_mut().expect("voice engine just created");
        (veh.engine.take_capture_rx(), veh.engine.take_playback_tx())
    };

    // Read audio processing config
    let (noise_suppression, echo_cancellation) = {
        let ve = state.voice_engine.lock();
        let config = ve.as_ref().map(|h| h.engine.config());
        (
            config.is_none_or(|c| c.noise_suppression),
            config.is_none_or(|c| c.echo_cancellation),
        )
    };

    // Set up voice packet receive channel
    let (voice_packet_tx, voice_packet_rx) = mpsc::channel(200);
    *state.voice_packet_tx.write() = Some(voice_packet_tx);

    // Speaker reference broadcast channel for AEC — receive loop sends mixed audio,
    // send loop receives it to feed the echo canceller.
    let (speaker_ref_tx, speaker_ref_rx) = broadcast::channel::<Vec<f32>>(50);

    // Spawn voice send loop
    let (send_shutdown_tx, send_shutdown_rx) = mpsc::channel::<()>(1);
    let send_app = app.clone();
    let send_public_key = identity.public_key.clone();
    let send_muted = Arc::clone(&muted_flag);
    let send_handle = tokio::spawn(voice_send_loop(
        capture_rx,
        transport,
        send_shutdown_rx,
        send_app,
        send_public_key,
        noise_suppression,
        echo_cancellation,
        send_muted,
        speaker_ref_rx,
    ));

    // Spawn voice receive loop
    let (recv_shutdown_tx, recv_shutdown_rx) = mpsc::channel::<()>(1);
    let recv_app = app.clone();
    let recv_public_key = identity.public_key.clone();
    let recv_deafened = Arc::clone(&deafened_flag);
    let recv_handle = tokio::spawn(voice_receive_loop(
        voice_packet_rx,
        playback_tx,
        recv_shutdown_rx,
        recv_app,
        recv_public_key,
        recv_deafened,
        speaker_ref_tx,
    ));

    // Take device error receiver and spawn device monitor loop
    let device_error_rx = {
        let mut ve = state.voice_engine.lock();
        ve.as_mut().and_then(|h| h.engine.take_device_error_rx())
    };
    let (monitor_shutdown_tx, monitor_shutdown_rx) = mpsc::channel::<()>(1);
    let monitor_handle = if let Some(error_rx) = device_error_rx {
        let monitor_app = app.clone();
        let monitor_state = state.inner().clone();
        Some(tokio::spawn(device_monitor_loop(
            error_rx,
            monitor_shutdown_rx,
            monitor_app,
            monitor_state,
        )))
    } else {
        None
    };

    // Store shutdown handles
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.send_loop_shutdown = Some(send_shutdown_tx);
            handle.send_loop_handle = Some(send_handle);
            handle.recv_loop_shutdown = Some(recv_shutdown_tx);
            handle.recv_loop_handle = Some(recv_handle);
            handle.device_monitor_shutdown = Some(monitor_shutdown_tx);
            handle.device_monitor_handle = monitor_handle;
        }
    }

    // Emit voice events to frontend
    let event = VoiceEvent::UserJoined {
        public_key: identity.public_key.clone(),
        display_name: identity.display_name,
    };
    let _ = app.emit("voice-event", &event);

    let quality_event = VoiceEvent::ConnectionQuality {
        quality: "good".to_string(),
    };
    let _ = app.emit("voice-event", &quality_event);

    let speaking_event = VoiceEvent::UserSpeaking {
        public_key: identity.public_key,
        speaking: false,
    };
    let _ = app.emit("voice-event", &speaking_event);

    Ok(())
}

/// Leave the current voice channel.
#[tauri::command]
pub async fn leave_voice(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let public_key = state_helpers::owner_key_or_default(state.inner());

    shutdown_voice_loops(&state).await;

    // Emit leave event
    let event = VoiceEvent::UserLeft { public_key };
    let _ = app.emit("voice-event", &event);

    Ok(())
}

/// Shut down voice send/receive loops, stop audio devices, and clear state.
///
/// Used by both `leave_voice` and lifecycle cleanup (logout, app exit).
pub(crate) async fn shutdown_voice_loops(state: &SharedState) {
    // Extract shutdown senders and join handles outside the lock
    let (send_tx, send_h, recv_tx, recv_h, monitor_tx, monitor_h) = {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            (
                handle.send_loop_shutdown.take(),
                handle.send_loop_handle.take(),
                handle.recv_loop_shutdown.take(),
                handle.recv_loop_handle.take(),
                handle.device_monitor_shutdown.take(),
                handle.device_monitor_handle.take(),
            )
        } else {
            (None, None, None, None, None, None)
        }
    };

    // Signal all loops to shut down
    if let Some(tx) = send_tx {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = recv_tx {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = monitor_tx {
        let _ = tx.send(()).await;
    }

    // Await all loop handles
    if let Some(h) = send_h {
        let _ = h.await;
    }
    if let Some(h) = recv_h {
        let _ = h.await;
    }
    if let Some(h) = monitor_h {
        let _ = h.await;
    }

    // Stop audio devices and clear engine
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
        }
        *ve = None;
    }

    // Clear voice packet channel
    *state.voice_packet_tx.write() = None;
}

/// Shut down send and receive loops only, leaving the engine and device monitor alive.
///
/// Used by `handle_device_swap` (called from within the monitor loop — cannot
/// await its own `JoinHandle`).
async fn shutdown_send_recv_only(state: &SharedState) {
    let (send_tx, send_h, recv_tx, recv_h) = {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            (
                handle.send_loop_shutdown.take(),
                handle.send_loop_handle.take(),
                handle.recv_loop_shutdown.take(),
                handle.recv_loop_handle.take(),
            )
        } else {
            (None, None, None, None)
        }
    };

    if let Some(tx) = send_tx {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = recv_tx {
        let _ = tx.send(()).await;
    }
    if let Some(h) = send_h {
        let _ = h.await;
    }
    if let Some(h) = recv_h {
        let _ = h.await;
    }

    *state.voice_packet_tx.write() = None;
}

/// Shut down voice loops AND device monitor but keep the engine alive (for device hot-swap).
///
/// Unlike `shutdown_voice_loops`, this does NOT stop audio devices or clear the engine.
/// Called from external commands (e.g. `set_audio_devices`) — NOT from within the monitor loop.
async fn shutdown_voice_loops_keep_engine(state: &SharedState) {
    let (send_tx, send_h, recv_tx, recv_h, monitor_tx, monitor_h) = {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            (
                handle.send_loop_shutdown.take(),
                handle.send_loop_handle.take(),
                handle.recv_loop_shutdown.take(),
                handle.recv_loop_handle.take(),
                handle.device_monitor_shutdown.take(),
                handle.device_monitor_handle.take(),
            )
        } else {
            (None, None, None, None, None, None)
        }
    };

    if let Some(tx) = send_tx {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = recv_tx {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = monitor_tx {
        let _ = tx.send(()).await;
    }
    if let Some(h) = send_h {
        let _ = h.await;
    }
    if let Some(h) = recv_h {
        let _ = h.await;
    }
    if let Some(h) = monitor_h {
        let _ = h.await;
    }

    *state.voice_packet_tx.write() = None;
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

/// Restart voice capture/playback and respawn send/receive loops.
///
/// Assumes loops are already shut down and capture/playback are stopped.
/// Used by hot-swap (device change mid-call) and could be used for reconnect.
fn restart_voice_loops(state: &SharedState, app: &tauri::AppHandle) -> Result<(), String> {
    let identity = state_helpers::current_identity(state)?;

    // Restart capture and playback, take channels
    let (
        capture_rx,
        playback_tx,
        channel_id,
        muted_flag,
        deafened_flag,
        noise_suppression,
        echo_cancellation,
    ) = {
        let mut ve = state.voice_engine.lock();
        let handle = ve.as_mut().ok_or("no active voice engine")?;

        handle
            .engine
            .start_capture()
            .map_err(|e| format!("failed to restart capture: {e}"))?;
        handle
            .engine
            .start_playback()
            .map_err(|e| format!("failed to restart playback: {e}"))?;

        let ns = handle.engine.config().noise_suppression;
        let ec = handle.engine.config().echo_cancellation;
        (
            handle.engine.take_capture_rx(),
            handle.engine.take_playback_tx(),
            handle.channel_id.clone(),
            Arc::clone(&handle.muted_flag),
            Arc::clone(&handle.deafened_flag),
            ns,
            ec,
        )
    };

    // Create new transport and try to connect
    let mut transport = rekindle_voice::transport::VoiceTransport::new(channel_id.clone());
    let route_blob = state_helpers::cached_route_blob(state, &channel_id);
    let api = state_helpers::veilid_api(state);
    if let (Some(blob), Some(api)) = (route_blob, api) {
        let sender_key = hex::decode(&identity.public_key).unwrap_or_default();
        if let Err(e) = transport.connect(api, &blob, sender_key) {
            tracing::warn!(error = %e, "hot-swap: voice transport reconnect failed");
        }
    }

    // New voice packet receive channel
    let (voice_packet_tx, voice_packet_rx) = mpsc::channel(200);
    *state.voice_packet_tx.write() = Some(voice_packet_tx);

    // New speaker reference broadcast channel
    let (speaker_ref_tx, speaker_ref_rx) = broadcast::channel::<Vec<f32>>(50);

    // Spawn send loop
    let (send_shutdown_tx, send_shutdown_rx) = mpsc::channel::<()>(1);
    let send_handle = tokio::spawn(voice_send_loop(
        capture_rx,
        transport,
        send_shutdown_rx,
        app.clone(),
        identity.public_key.clone(),
        noise_suppression,
        echo_cancellation,
        Arc::clone(&muted_flag),
        speaker_ref_rx,
    ));

    // Spawn receive loop
    let (recv_shutdown_tx, recv_shutdown_rx) = mpsc::channel::<()>(1);
    let recv_handle = tokio::spawn(voice_receive_loop(
        voice_packet_rx,
        playback_tx,
        recv_shutdown_rx,
        app.clone(),
        identity.public_key.clone(),
        Arc::clone(&deafened_flag),
        speaker_ref_tx,
    ));

    // Respawn device monitor with fresh error channel
    let device_error_rx = {
        let mut ve = state.voice_engine.lock();
        ve.as_mut()
            .map(|h| h.engine.refresh_device_error_channels())
    };
    let (monitor_shutdown_tx, monitor_shutdown_rx) = mpsc::channel::<()>(1);
    let monitor_handle = if let Some(error_rx) = device_error_rx {
        let monitor_app = app.clone();
        let monitor_state = state.clone();
        Some(tokio::spawn(device_monitor_loop(
            error_rx,
            monitor_shutdown_rx,
            monitor_app,
            monitor_state,
        )))
    } else {
        None
    };

    // Store new shutdown handles
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.send_loop_shutdown = Some(send_shutdown_tx);
            handle.send_loop_handle = Some(send_handle);
            handle.recv_loop_shutdown = Some(recv_shutdown_tx);
            handle.recv_loop_handle = Some(recv_handle);
            handle.device_monitor_shutdown = Some(monitor_shutdown_tx);
            handle.device_monitor_handle = monitor_handle;
        }
    }

    Ok(())
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
        shutdown_voice_loops_keep_engine(&state).await;

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

        // Restart with new devices — the engine will pick up device names
        // from VoiceConfig which reads from preferences on next start_capture/start_playback
        restart_voice_loops(&state, &app)?;
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

// ── Device Monitor Loop ──────────────────────────────────────────────────

/// Monitors audio device availability via error callbacks and periodic polling.
///
/// On device error (cpal callback) or when a selected device disappears from
/// the enumerated list, performs a hot-swap to the default device and emits
/// `VoiceEvent::DeviceChanged` + `NotificationEvent::SystemAlert`.
#[allow(clippy::too_many_lines)]
async fn device_monitor_loop(
    mut device_error_rx: mpsc::Receiver<String>,
    mut shutdown_rx: mpsc::Receiver<()>,
    app: tauri::AppHandle,
    state: SharedState,
) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!("device monitor loop started");

    loop {
        tokio::select! {
            biased;

            _ = shutdown_rx.recv() => {
                tracing::info!("device monitor loop: shutdown signal received");
                break;
            }

            Some(error_msg) = device_error_rx.recv() => {
                tracing::warn!(error = %error_msg, "device monitor: cpal stream error detected");

                // Determine which device type errored based on the message prefix
                let device_type = if error_msg.starts_with("input:") {
                    "input"
                } else {
                    "output"
                };

                // Hot-swap to default devices. restart_voice_loops spawns a new
                // monitor, so this instance must exit after the swap.
                if let Err(e) = handle_device_swap(&app, &state, device_type, "disconnected").await {
                    tracing::error!(error = %e, "device monitor: hot-swap failed after error");
                }
                break;
            }

            _ = tick.tick() => {
                // Read currently selected device names from the engine
                let (input_device, output_device) = {
                    let ve = state.voice_engine.lock();
                    match ve.as_ref() {
                        Some(handle) => {
                            let cfg = handle.engine.config();
                            (cfg.input_device.clone(), cfg.output_device.clone())
                        }
                        None => break, // Engine gone — stop monitoring
                    }
                };

                // If using defaults (None), skip check — OS handles default routing
                if input_device.is_none() && output_device.is_none() {
                    continue;
                }

                // Enumerate current devices
                let devices = match rekindle_voice::capture::enumerate_audio_devices() {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::debug!(error = %e, "device monitor: enumeration failed");
                        continue;
                    }
                };

                let input_names: Vec<&str> = devices
                    .input_devices
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect();
                let output_names: Vec<&str> = devices
                    .output_devices
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect();

                // Check if selected input device disappeared
                if let Some(ref name) = input_device {
                    if !input_names.contains(&name.as_str()) {
                        tracing::warn!(device = %name, "device monitor: selected input device disappeared");
                        if let Err(e) = handle_device_swap(&app, &state, "input", "disconnected").await {
                            tracing::error!(error = %e, "device monitor: input hot-swap failed");
                        }
                        // restart_voice_loops spawned a new monitor — this one must exit
                        break;
                    }
                }

                // Check if selected output device disappeared
                if let Some(ref name) = output_device {
                    if !output_names.contains(&name.as_str()) {
                        tracing::warn!(device = %name, "device monitor: selected output device disappeared");
                        if let Err(e) = handle_device_swap(&app, &state, "output", "disconnected").await {
                            tracing::error!(error = %e, "device monitor: output hot-swap failed");
                        }
                        // restart_voice_loops spawned a new monitor — this one must exit
                        break;
                    }
                }
            }
        }
    }

    tracing::info!("device monitor loop exited");
}

/// Perform a device hot-swap to defaults when a device disappears.
///
/// Only shuts down send/recv loops (NOT the device monitor — this is called
/// FROM the monitor loop, so awaiting its own handle would deadlock).
/// After this returns, the caller (`device_monitor_loop`) MUST break out of its
/// loop — `restart_voice_loops` spawns a fresh monitor to replace it.
async fn handle_device_swap(
    app: &tauri::AppHandle,
    state: &SharedState,
    device_type: &str,
    reason: &str,
) -> Result<(), String> {
    // Check voice engine is still active
    {
        let ve = state.voice_engine.lock();
        if ve.is_none() {
            return Ok(());
        }
    }

    // Shut down send/recv loops only — NOT the monitor (we ARE the monitor)
    shutdown_send_recv_only(state).await;

    // Stop capture/playback and switch to default devices
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
            handle.engine.set_devices(None, None);
        }
    }

    // Restart with default devices — this spawns new send/recv loops AND a new monitor
    restart_voice_loops(state, app)?;

    // Emit DeviceChanged event
    let event = VoiceEvent::DeviceChanged {
        device_type: device_type.to_string(),
        device_name: "default".to_string(),
        reason: reason.to_string(),
    };
    let _ = app.emit("voice-event", &event);

    // Emit notification
    let notification = NotificationEvent::SystemAlert {
        title: "Audio Device Disconnected".to_string(),
        body: format!("Your {device_type} device was disconnected. Switched to default device."),
    };
    let _ = app.emit("notification-event", &notification);

    tracing::info!(
        device_type,
        reason,
        "device monitor: hot-swapped to default device"
    );

    Ok(())
}

// ── Send Loop ────────────────────────────────────────────────────────────

/// Voice send loop: drains `capture_rx`, runs `AudioProcessor`, encodes with Opus, sends via transport.
///
/// This task owns the `VoiceTransport` and runs until a shutdown signal is received
/// or the capture channel closes. On exit it disconnects the transport.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn voice_send_loop(
    capture_rx: Option<mpsc::Receiver<Vec<f32>>>,
    mut transport: rekindle_voice::transport::VoiceTransport,
    mut shutdown_rx: mpsc::Receiver<()>,
    app: tauri::AppHandle,
    public_key: String,
    noise_suppression: bool,
    echo_cancellation: bool,
    muted_flag: Arc<AtomicBool>,
    mut speaker_ref_rx: broadcast::Receiver<Vec<f32>>,
) {
    let Some(mut capture_rx) = capture_rx else {
        tracing::warn!("voice send loop started without capture_rx — exiting");
        return;
    };

    let sample_rate: u32 = 48000;
    let channels: u16 = 1;
    let frame_size: usize = 960; // 20ms at 48kHz

    let mut codec = match rekindle_voice::codec::OpusCodec::new(sample_rate, channels, frame_size) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "voice send loop: failed to create Opus codec");
            return;
        }
    };

    let frame_duration_ms = u32::try_from(frame_size).unwrap_or(960) * 1000 / sample_rate;
    let mut audio_processor = rekindle_voice::audio_processing::AudioProcessor::new(
        noise_suppression,
        echo_cancellation,
        0.02, // vad_threshold
        300,  // vad_hold_ms
        frame_duration_ms,
    );

    let mut sequence: u32 = 0;
    let mut was_speaking = false;

    // Accumulation buffer: cpal may deliver chunks that don't align with
    // Opus frame boundaries, so we accumulate samples here.
    let mut pcm_buffer: Vec<f32> = Vec::with_capacity(frame_size * 2);

    // Connection quality tracking
    let mut packets_sent: u64 = 0;
    let mut send_failures: u64 = 0;
    let mut last_quality_report = Instant::now();

    tracing::info!("voice send loop started");

    loop {
        tokio::select! {
            biased;

            _ = shutdown_rx.recv() => {
                tracing::info!("voice send loop: shutdown signal received");
                break;
            }

            maybe_samples = capture_rx.recv() => {
                let Some(samples) = maybe_samples else {
                    tracing::info!("voice send loop: capture channel closed");
                    break;
                };

                pcm_buffer.extend_from_slice(&samples);

                while pcm_buffer.len() >= frame_size {
                    let frame_samples: Vec<f32> =
                        pcm_buffer.drain(..frame_size).collect();

                    // Skip processing when muted — still drain capture to avoid backpressure
                    if muted_flag.load(Ordering::Relaxed) {
                        if was_speaking {
                            was_speaking = false;
                            let event = VoiceEvent::UserSpeaking {
                                public_key: public_key.clone(),
                                speaking: false,
                            };
                            let _ = app.emit("voice-event", &event);
                        }
                        continue;
                    }

                    // Drain speaker reference frames for AEC
                    let mut latest_speaker_ref: Option<Vec<f32>> = None;
                    while let Ok(ref_frame) = speaker_ref_rx.try_recv() {
                        audio_processor.feed_speaker_reference(&ref_frame);
                        latest_speaker_ref = Some(ref_frame);
                    }

                    // Run audio processor (AEC + denoise + VAD)
                    let processed = audio_processor.process_capture(
                        &frame_samples,
                        latest_speaker_ref.as_deref(),
                    );

                    // Emit speaking state change to frontend
                    if processed.is_speech != was_speaking {
                        was_speaking = processed.is_speech;
                        let event = VoiceEvent::UserSpeaking {
                            public_key: public_key.clone(),
                            speaking: processed.is_speech,
                        };
                        let _ = app.emit("voice-event", &event);
                    }

                    // Only encode and send if speaking (VAD gate)
                    if !processed.is_speech {
                        continue;
                    }

                    // Encode the processed PCM frame with Opus
                    let mut encoded = match codec.encode(&processed.samples) {
                        Ok(frame) => frame,
                        Err(e) => {
                            tracing::warn!(error = %e, "voice send loop: Opus encode failed");
                            continue;
                        }
                    };

                    encoded.sequence = sequence;
                    encoded.timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
                        .unwrap_or(0);
                    sequence = sequence.wrapping_add(1);

                    if transport.is_connected() {
                        if let Err(e) = transport.send(&encoded).await {
                            tracing::debug!(error = %e, "voice send loop: transport send failed");
                            send_failures += 1;
                        }
                        packets_sent += 1;
                    }

                    // Periodic connection quality report (every 5s)
                    if last_quality_report.elapsed() >= Duration::from_secs(5) {
                        let loss_pct = if packets_sent > 0 {
                            #[allow(clippy::cast_precision_loss)]
                            let pct = (send_failures as f64 / packets_sent as f64) * 100.0;
                            pct
                        } else {
                            0.0
                        };
                        let quality = if loss_pct < 5.0 {
                            "good"
                        } else if loss_pct < 15.0 {
                            "fair"
                        } else {
                            "poor"
                        };
                        let event = VoiceEvent::ConnectionQuality {
                            quality: quality.to_string(),
                        };
                        let _ = app.emit("voice-event", &event);

                        // Update Opus FEC based on measured loss
                        #[allow(clippy::cast_possible_truncation)]
                        let loss_i32 = (loss_pct as i32).clamp(0, 100);
                        let _ = codec.set_packet_loss_perc(loss_i32);

                        packets_sent = 0;
                        send_failures = 0;
                        last_quality_report = Instant::now();
                    }
                }
            }
        }
    }

    // Clean up: disconnect transport
    if let Err(e) = transport.disconnect() {
        tracing::warn!(error = %e, "voice send loop: transport disconnect failed");
    }

    if was_speaking {
        let event = VoiceEvent::UserSpeaking {
            public_key,
            speaking: false,
        };
        let _ = app.emit("voice-event", &event);
    }

    tracing::info!("voice send loop exited");
}

// ── Receive Loop ─────────────────────────────────────────────────────────

/// Per-participant decoder state in the receive loop.
struct ParticipantDecoder {
    codec: rekindle_voice::codec::OpusCodec,
    jitter_buffer: rekindle_voice::jitter::JitterBuffer,
    is_speaking: bool,
    last_packet_time: Instant,
}

/// Voice receive loop: receives `VoicePacket`s, decodes per-participant, mixes, sends to playback.
///
/// Runs on a 20ms tick (50Hz) cadence that drives decode/mix/playback independently
/// of packet arrival timing.
#[allow(clippy::too_many_lines)]
async fn voice_receive_loop(
    mut packet_rx: mpsc::Receiver<rekindle_voice::transport::VoicePacket>,
    playback_tx: Option<mpsc::Sender<Vec<f32>>>,
    mut shutdown_rx: mpsc::Receiver<()>,
    app: tauri::AppHandle,
    our_public_key: String,
    deafened_flag: Arc<AtomicBool>,
    speaker_ref_tx: broadcast::Sender<Vec<f32>>,
) {
    let Some(playback_tx) = playback_tx else {
        tracing::warn!("voice receive loop started without playback_tx — exiting");
        return;
    };

    let sample_rate: u32 = 48000;
    let channels: u16 = 1;
    let frame_size: usize = 960;
    let jitter_buffer_ms: u32 = 200;

    let our_key_bytes = hex::decode(&our_public_key).unwrap_or_default();

    let mut participants: HashMap<Vec<u8>, ParticipantDecoder> = HashMap::new();
    let audio_mixer = rekindle_voice::mixer::AudioMixer::new(channels);

    // 20ms tick drives the decode/mix/playback cadence
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Connection quality tracking
    let mut packets_received: u64 = 0;
    let mut last_quality_check = Instant::now();

    tracing::info!("voice receive loop started");

    loop {
        tokio::select! {
            biased;

            _ = shutdown_rx.recv() => {
                tracing::info!("voice receive loop: shutdown signal received");
                break;
            }

            // Receive incoming voice packets and push into per-participant jitter buffers
            Some(packet) = packet_rx.recv() => {
                // Skip our own packets
                if packet.sender_key == our_key_bytes {
                    continue;
                }

                packets_received += 1;

                let sender_key = packet.sender_key.clone();

                // Get or create participant decoder
                if !participants.contains_key(&sender_key) {
                    match rekindle_voice::codec::OpusCodec::new(sample_rate, channels, frame_size) {
                        Ok(codec) => {
                            let sender_hex = hex::encode(&sender_key);
                            tracing::info!(peer = %sender_hex, "new voice participant");

                            // Emit UserJoined event
                            let event = VoiceEvent::UserJoined {
                                public_key: sender_hex.clone(),
                                display_name: sender_hex,
                            };
                            let _ = app.emit("voice-event", &event);

                            participants.insert(sender_key.clone(), ParticipantDecoder {
                                codec,
                                jitter_buffer: rekindle_voice::jitter::JitterBuffer::new(jitter_buffer_ms),
                                is_speaking: false,
                                last_packet_time: Instant::now(),
                            });
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to create decoder for participant");
                            continue;
                        }
                    }
                }

                if let Some(participant) = participants.get_mut(&sender_key) {
                    participant.jitter_buffer.push(packet);
                    participant.last_packet_time = Instant::now();

                    // Track speaking state
                    if !participant.is_speaking {
                        participant.is_speaking = true;
                        let event = VoiceEvent::UserSpeaking {
                            public_key: hex::encode(&sender_key),
                            speaking: true,
                        };
                        let _ = app.emit("voice-event", &event);
                    }
                }
            }

            // 20ms tick: pop from jitter buffers, decode, mix, send to playback
            _ = tick.tick() => {
                let mut streams: Vec<(String, Vec<f32>)> = Vec::new();

                for (key, participant) in &mut participants {
                    let decoded = match participant.jitter_buffer.pop() {
                        Some(packet) => {
                            let frame = rekindle_voice::codec::EncodedFrame {
                                data: packet.audio_data,
                                timestamp: packet.timestamp,
                                sequence: packet.sequence,
                            };
                            match participant.codec.decode(&frame) {
                                Ok(decoded) => decoded.samples,
                                Err(e) => {
                                    tracing::trace!(error = %e, "decode failed — using PLC");
                                    participant.codec.decode_plc()
                                        .map_or_else(|_| vec![0.0; frame_size], |d| d.samples)
                                }
                            }
                        }
                        None => {
                            // No packet available — try FEC if next packet exists, else PLC
                            if participant.last_packet_time.elapsed() < Duration::from_secs(2) {
                                if let Some(next_data) = participant.jitter_buffer.peek_next_audio_data() {
                                    // Use FEC data from the next packet to recover this one
                                    participant.codec.decode_fec(next_data)
                                        .map_or_else(|_| vec![0.0; frame_size], |d| d.samples)
                                } else {
                                    participant.codec.decode_plc()
                                        .map_or_else(|_| vec![0.0; frame_size], |d| d.samples)
                                }
                            } else {
                                continue; // Participant timed out, skip
                            }
                        }
                    };

                    streams.push((hex::encode(key), decoded));
                }

                if !streams.is_empty() {
                    // Mix all participant streams
                    let refs: Vec<(&str, &[f32])> = streams
                        .iter()
                        .map(|(id, samples)| (id.as_str(), samples.as_slice()))
                        .collect();
                    let mixed = audio_mixer.mix(&refs);

                    if !mixed.is_empty() {
                        // Broadcast mixed audio as speaker reference for AEC
                        // (before applying deafen — AEC needs what the speakers actually output)
                        let _ = speaker_ref_tx.send(mixed.clone());

                        // When deafened, send silence to keep the playback stream alive
                        let output = if deafened_flag.load(Ordering::Relaxed) {
                            vec![0.0f32; mixed.len()]
                        } else {
                            mixed
                        };
                        if playback_tx.try_send(output).is_err() {
                            tracing::trace!("playback channel full — dropping mixed frame");
                        }
                    }
                }

                // Clean up timed-out participants (>5s since last packet)
                let timeout_keys: Vec<Vec<u8>> = participants
                    .iter()
                    .filter(|(_, p)| p.last_packet_time.elapsed() > Duration::from_secs(5))
                    .map(|(k, _)| k.clone())
                    .collect();

                for key in timeout_keys {
                    if let Some(participant) = participants.remove(&key) {
                        let peer_hex = hex::encode(&key);
                        tracing::info!(peer = %peer_hex, "voice participant timed out");

                        if participant.is_speaking {
                            let event = VoiceEvent::UserSpeaking {
                                public_key: peer_hex.clone(),
                                speaking: false,
                            };
                            let _ = app.emit("voice-event", &event);
                        }

                        let event = VoiceEvent::UserLeft {
                            public_key: peer_hex,
                        };
                        let _ = app.emit("voice-event", &event);
                    }
                }

                // Update speaking state for participants who stopped sending
                for (key, participant) in &mut participants {
                    if participant.is_speaking
                        && participant.last_packet_time.elapsed() > Duration::from_millis(500)
                    {
                        participant.is_speaking = false;
                        let event = VoiceEvent::UserSpeaking {
                            public_key: hex::encode(key),
                            speaking: false,
                        };
                        let _ = app.emit("voice-event", &event);
                    }
                }

                // Periodic quality report
                if last_quality_check.elapsed() >= Duration::from_secs(5) {
                    tracing::debug!(
                        participants = participants.len(),
                        packets_received,
                        "voice receive loop stats"
                    );
                    packets_received = 0;
                    last_quality_check = Instant::now();
                }
            }
        }
    }

    // Emit UserLeft for all remaining participants
    for (key, participant) in &participants {
        let peer_hex = hex::encode(key);
        if participant.is_speaking {
            let event = VoiceEvent::UserSpeaking {
                public_key: peer_hex.clone(),
                speaking: false,
            };
            let _ = app.emit("voice-event", &event);
        }
        let event = VoiceEvent::UserLeft {
            public_key: peer_hex,
        };
        let _ = app.emit("voice-event", &event);
    }

    tracing::info!("voice receive loop exited");
}
