//! Voice session lifecycle: join, restart, shared setup helpers.
//!
//! Deduplicates the spawn logic between `start_session` (called by the
//! `join_voice_channel` command) and `restart_loops` (called by device hot-swap).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tauri::Emitter;
use tauri_plugin_store::StoreExt;
use tokio::sync::{broadcast, mpsc};

use crate::channels::VoiceEvent;
use crate::state::{SharedState, VoiceEngineHandle};
use crate::state_helpers;

use super::{device_monitor, receive_loop, send_loop};

/// Channels and config needed to spawn the voice loops.
struct LoopBundle {
    capture_rx: Option<mpsc::Receiver<Vec<f32>>>,
    playback_tx: Option<mpsc::Sender<Vec<f32>>>,
    noise_suppression: bool,
    echo_cancellation: bool,
}

// ── Public API ──────────────────────────────────────────────────────────

/// Called by `commands::voice::join_voice_channel` (the thin IPC wrapper).
pub(crate) fn start_session(
    channel_id: &str,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<(), String> {
    check_not_in_call(state, channel_id)?;

    let identity = state_helpers::current_identity(state)?;
    let prefs = load_audio_prefs(app);

    let (muted_flag, deafened_flag) = init_engine(state, &prefs, channel_id)?;
    start_audio_devices(state)?;

    let transport = create_transport(state, &identity.public_key, channel_id);
    let bundle = take_channels_and_config(state)?;

    spawn_loops(
        state,
        app,
        &identity.public_key,
        bundle,
        transport,
        &muted_flag,
        &deafened_flag,
    );

    emit_join_events(app, &identity.public_key, &identity.display_name);

    Ok(())
}

/// Restart capture/playback and respawn send/receive/monitor loops.
///
/// Assumes loops are already shut down and capture/playback are stopped.
/// Used by device hot-swap (`device_monitor::handle_device_swap`).
pub(crate) fn restart_loops(state: &SharedState, app: &tauri::AppHandle) -> Result<(), String> {
    let identity = state_helpers::current_identity(state)?;

    restart_audio_devices(state)?;

    let channel_id = {
        let ve = state.voice_engine.lock();
        ve.as_ref()
            .ok_or("no active voice engine")?
            .channel_id
            .clone()
    };

    let transport = create_transport(state, &identity.public_key, &channel_id);
    let bundle = take_channels_and_config(state)?;
    let (muted_flag, deafened_flag) = clone_flags(state)?;

    spawn_loops(
        state,
        app,
        &identity.public_key,
        bundle,
        transport,
        &muted_flag,
        &deafened_flag,
    );

    Ok(())
}

// ── Private Helpers ─────────────────────────────────────────────────────

fn check_not_in_call(state: &SharedState, channel_id: &str) -> Result<(), String> {
    let ve = state.voice_engine.lock();
    if let Some(ref handle) = *ve {
        if handle.channel_id == channel_id {
            return Err("already in this voice channel".to_string());
        }
        return Err(format!("already in voice channel {}", handle.channel_id));
    }
    Ok(())
}

fn load_audio_prefs(app: &tauri::AppHandle) -> crate::commands::settings::Preferences {
    app.store("preferences.json")
        .ok()
        .and_then(|store| store.get("preferences"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn init_engine(
    state: &SharedState,
    prefs: &crate::commands::settings::Preferences,
    channel_id: &str,
) -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), String> {
    let muted_flag = Arc::new(AtomicBool::new(false));
    let deafened_flag = Arc::new(AtomicBool::new(false));

    let config = rekindle_voice::VoiceConfig {
        input_device: prefs.input_device.clone(),
        output_device: prefs.output_device.clone(),
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
        channel_id: channel_id.to_string(),
        muted_flag: Arc::clone(&muted_flag),
        deafened_flag: Arc::clone(&deafened_flag),
    });

    Ok((muted_flag, deafened_flag))
}

fn start_audio_devices(state: &SharedState) -> Result<(), String> {
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
    Ok(())
}

fn restart_audio_devices(state: &SharedState) -> Result<(), String> {
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

    Ok(())
}

fn create_transport(
    state: &SharedState,
    public_key: &str,
    channel_id: &str,
) -> rekindle_voice::transport::VoiceTransport {
    let mut transport = rekindle_voice::transport::VoiceTransport::new(channel_id.to_string());

    let route_blob = state_helpers::cached_route_blob(state, channel_id);
    let api = state_helpers::veilid_api(state);

    if let (Some(blob), Some(api)) = (route_blob, api) {
        let sender_key = hex::decode(public_key).unwrap_or_default();
        if let Err(e) = transport.connect(api, &blob, sender_key) {
            tracing::warn!(error = %e, channel = %channel_id, "voice transport connect failed — audio only local");
        }
    }

    transport
}

fn take_channels_and_config(state: &SharedState) -> Result<LoopBundle, String> {
    let mut ve = state.voice_engine.lock();
    let handle = ve.as_mut().ok_or("no active voice engine")?;

    let ns = handle.engine.config().noise_suppression;
    let ec = handle.engine.config().echo_cancellation;

    Ok(LoopBundle {
        capture_rx: handle.engine.take_capture_rx(),
        playback_tx: handle.engine.take_playback_tx(),
        noise_suppression: ns,
        echo_cancellation: ec,
    })
}

fn clone_flags(state: &SharedState) -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), String> {
    let ve = state.voice_engine.lock();
    let handle = ve.as_ref().ok_or("no active voice engine")?;
    Ok((
        Arc::clone(&handle.muted_flag),
        Arc::clone(&handle.deafened_flag),
    ))
}

/// Spawn send, receive, and device monitor loops. Store handles in `VoiceEngineHandle`.
fn spawn_loops(
    state: &SharedState,
    app: &tauri::AppHandle,
    public_key: &str,
    bundle: LoopBundle,
    transport: rekindle_voice::transport::VoiceTransport,
    muted_flag: &Arc<AtomicBool>,
    deafened_flag: &Arc<AtomicBool>,
) {
    // Set up voice packet receive channel
    let (voice_packet_tx, voice_packet_rx) = mpsc::channel(200);
    *state.voice_packet_tx.write() = Some(voice_packet_tx);

    // Speaker reference broadcast channel for AEC
    let (speaker_ref_tx, speaker_ref_rx) = broadcast::channel::<Vec<f32>>(50);

    // Spawn voice send loop
    let (send_shutdown_tx, send_shutdown_rx) = mpsc::channel::<()>(1);
    let send_handle = tokio::spawn(send_loop::run(send_loop::VoiceSendParams {
        capture_rx: bundle.capture_rx,
        transport,
        shutdown_rx: send_shutdown_rx,
        app: app.clone(),
        public_key: public_key.to_string(),
        noise_suppression: bundle.noise_suppression,
        echo_cancellation: bundle.echo_cancellation,
        muted_flag: Arc::clone(muted_flag),
        speaker_ref_rx,
    }));

    // Spawn voice receive loop
    let (recv_shutdown_tx, recv_shutdown_rx) = mpsc::channel::<()>(1);
    let recv_handle = tokio::spawn(receive_loop::run(receive_loop::VoiceReceiveParams {
        packet_rx: voice_packet_rx,
        playback_tx: bundle.playback_tx,
        shutdown_rx: recv_shutdown_rx,
        app: app.clone(),
        our_public_key: public_key.to_string(),
        deafened_flag: Arc::clone(deafened_flag),
        speaker_ref_tx,
    }));

    // Take device error receiver and spawn device monitor loop.
    // On first join, `take_device_error_rx()` returns the original receiver.
    // On restart (hot-swap), the original was consumed — `refresh_device_error_channels()`
    // creates a fresh pair and returns the new receiver.
    let device_error_rx = {
        let mut ve = state.voice_engine.lock();
        ve.as_mut().and_then(|h| {
            h.engine
                .take_device_error_rx()
                .or_else(|| Some(h.engine.refresh_device_error_channels()))
        })
    };
    let (monitor_shutdown_tx, monitor_shutdown_rx) = mpsc::channel::<()>(1);
    let monitor_handle = device_error_rx.map(|error_rx| {
        tokio::spawn(device_monitor::run(device_monitor::DeviceMonitorParams {
            device_error_rx: error_rx,
            shutdown_rx: monitor_shutdown_rx,
            app: app.clone(),
            state: state.clone(),
        }))
    });

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
}

fn emit_join_events(app: &tauri::AppHandle, public_key: &str, display_name: &str) {
    let event = VoiceEvent::UserJoined {
        public_key: public_key.to_string(),
        display_name: display_name.to_string(),
    };
    let _ = app.emit("voice-event", &event);

    let quality_event = VoiceEvent::ConnectionQuality {
        quality: "good".to_string(),
    };
    let _ = app.emit("voice-event", &quality_event);

    let speaking_event = VoiceEvent::UserSpeaking {
        public_key: public_key.to_string(),
        speaking: false,
    };
    let _ = app.emit("voice-event", &speaking_event);
}
