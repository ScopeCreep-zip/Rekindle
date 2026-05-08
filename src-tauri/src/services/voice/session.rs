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
pub(crate) async fn start_session(
    channel_id: &str,
    community_id: Option<&str>,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<(), String> {
    check_not_in_call(state, channel_id)?;

    let identity = state_helpers::current_identity(state)?;
    let prefs = load_audio_prefs(app);

    let (muted_flag, deafened_flag) = init_engine(state, &prefs, channel_id, community_id)?;
    start_audio_devices(state)?;

    let transport = create_transport(state, &identity.public_key, channel_id, community_id);
    let shared_transport = std::sync::Arc::new(tokio::sync::Mutex::new(transport));

    // Store shared transport on VoiceEngineHandle for VoiceJoin/Leave + MCU access
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.transport = Arc::clone(&shared_transport);
        }
    }

    let bundle = take_channels_and_config(state)?;

    // Load member display names for voice participant identification
    let voice_community_id = {
        let ve = state.voice_engine.lock();
        ve.as_ref().and_then(|h| h.community_id.clone())
    };
    let member_names = load_community_member_names(state, voice_community_id.as_deref()).await;

    spawn_loops(
        state,
        app,
        &identity.public_key,
        bundle,
        &shared_transport,
        &muted_flag,
        &deafened_flag,
        member_names,
    );

    emit_join_events(app, &identity.public_key, &identity.display_name);

    // Backend-authoritative `active_call_type` so every frontend (Tauri GUI,
    // CLI, future TUI) mirrors the same state without per-frontend branching.
    // Fixes C1: VideoCallPanel never mounted because activeCallType was null
    // — the prior frontend-only set in voice.handlers.ts had no equivalent
    // for non-Tauri clients. The frontend listens for this event and sets
    // its store; the manual set in handleJoinVoice goes away.
    let local_joined = VoiceEvent::LocalJoined {
        channel_id: channel_id.to_string(),
        active_call_type: if community_id.is_some() { "community" } else { "dm" }.to_string(),
    };
    let _ = app.emit("voice-event", &local_joined);

    // Architecture §10.6 line 4084 — broadcast our decode capabilities
    // so other peers cap their VP9 sender at the lowest common
    // denominator. Only meaningful in community voice channels (no
    // peers in DM voice). Best-effort; failures are logged but never
    // block the join.
    if let Some(community_id) = community_id {
        if let Err(e) = broadcast_media_capabilities(state, community_id, channel_id) {
            tracing::warn!(error = %e, "MediaCapabilities broadcast failed");
        }
    }

    Ok(())
}

/// Architecture §10.6 line 4084 — capability negotiation. Build a
/// `ControlPayload::MediaCapabilities` with our interim defaults and
/// gossip it to the community so senders can pick the lowest common
/// resolution + framerate every connected peer can decode.
fn broadcast_media_capabilities(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    let caps = rekindle_video::MediaCapabilities::interim_default();
    let envelope = CommunityEnvelope::Control(ControlPayload::MediaCapabilities {
        channel_id: channel_id.to_string(),
        max_pixel_count: caps.max_pixel_count,
        max_fps: caps.max_fps,
        codecs: caps.codecs,
    });
    crate::services::community::send_to_mesh(state, community_id, &envelope)
}

/// Restart capture/playback and respawn send/receive/monitor loops.
///
/// Assumes loops are already shut down and capture/playback are stopped.
/// Used by device hot-swap (`device_monitor::handle_device_swap`).
///
/// Reuses the existing shared transport (preserves all connected peers
/// from VoiceJoin gossip). Only creates a new transport if none exists.
pub(crate) async fn restart_loops(
    state: &SharedState,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let identity = state_helpers::current_identity(state)?;

    restart_audio_devices(state)?;

    // Reuse existing shared transport — preserves all connected peers added
    // via VoiceJoin gossip. Creating a new transport would lose them.
    let shared_transport = {
        let ve = state.voice_engine.lock();
        ve.as_ref()
            .ok_or("no active voice engine")?
            .transport
            .clone()
    };

    let bundle = take_channels_and_config(state)?;
    let (muted_flag, deafened_flag) = clone_flags(state)?;

    let voice_community_id = {
        let ve = state.voice_engine.lock();
        ve.as_ref().and_then(|h| h.community_id.clone())
    };
    let member_names = load_community_member_names(state, voice_community_id.as_deref()).await;

    spawn_loops(
        state,
        app,
        &identity.public_key,
        bundle,
        &shared_transport,
        &muted_flag,
        &deafened_flag,
        member_names,
    );

    Ok(())
}

/// Start the MCU mixing loop (called when this peer becomes voice host).
///
/// The MCU loop receives incoming voice packets, decodes per-sender, mixes
/// per-recipient (excluding their own audio), re-encodes, and sends.
pub(crate) fn start_mcu_loop(state: &SharedState) -> Result<(), String> {
    let identity = state_helpers::current_identity(state)?;
    let our_key_bytes = hex::decode(&identity.public_key).unwrap_or_default();

    // Get the shared transport from VoiceEngineHandle (already initialized with peers)
    let transport = {
        let ve = state.voice_engine.lock();
        ve.as_ref()
            .ok_or("no active voice engine")?
            .transport
            .clone()
    };

    // MCU receives packets on a separate channel
    let (mcu_packet_tx, mcu_packet_rx) = mpsc::channel(200);
    // Store the MCU packet sender so the dispatch loop can forward packets to it
    *state.voice_packet_tx.write() = Some(mcu_packet_tx);

    let (mcu_shutdown_tx, mcu_shutdown_rx) = mpsc::channel::<()>(1);
    let mcu_handle = tokio::spawn(super::mcu_loop::run(super::mcu_loop::McuParams {
        transport,
        packet_rx: mcu_packet_rx,
        shutdown_rx: mcu_shutdown_rx,
        our_key_bytes,
    }));

    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.mcu_loop_shutdown = Some(mcu_shutdown_tx);
            handle.mcu_loop_handle = Some(mcu_handle);
        }
    }

    tracing::info!("MCU loop started — this peer is the voice host");
    Ok(())
}

/// Stop the MCU mixing loop (called when another peer becomes voice host,
/// or we leave the voice channel).
pub(crate) async fn stop_mcu_loop(state: &SharedState) {
    let (mcu_tx, mcu_h) = {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            (
                handle.mcu_loop_shutdown.take(),
                handle.mcu_loop_handle.take(),
            )
        } else {
            (None, None)
        }
    };
    if let Some(tx) = mcu_tx {
        let _ = tx.send(()).await;
    }
    if let Some(h) = mcu_h {
        let _ = h.await;
    }
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
    community_id: Option<&str>,
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
        transport: std::sync::Arc::new(tokio::sync::Mutex::new(
            rekindle_voice::transport::VoiceTransport::new(channel_id.to_string()),
        )),
        send_loop_shutdown: None,
        send_loop_handle: None,
        recv_loop_shutdown: None,
        recv_loop_handle: None,
        device_monitor_shutdown: None,
        device_monitor_handle: None,
        mcu_loop_shutdown: None,
        mcu_loop_handle: None,
        channel_id: channel_id.to_string(),
        community_id: community_id.map(String::from),
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
    community_id: Option<&str>,
) -> rekindle_voice::transport::VoiceTransport {
    let mut transport = rekindle_voice::transport::VoiceTransport::new(channel_id.to_string());

    let api = state_helpers::veilid_api(state);
    let sender_key = hex::decode(public_key).unwrap_or_default();

    if let Some(api) = api {
        if community_id.is_some() {
            // Community voice: peers arrive dynamically via VoiceJoin gossip.
            // Just initialize the transport — don't try to connect to a single peer.
            transport.init(api, sender_key);
        } else {
            // 1:1 DM voice: single remote peer looked up by channel_id.
            let route_blob = state_helpers::cached_route_blob(state, channel_id);
            if let Some(blob) = route_blob {
                if let Err(e) = transport.connect(api, &blob, sender_key) {
                    tracing::warn!(error = %e, channel = %channel_id, "voice transport connect failed — audio only local");
                }
            } else {
                transport.init(api, sender_key);
            }
        }
    }

    // Architecture §10.3 + §26 W26 — install the signing key so every
    // outbound packet is signed and receivers can authenticate the
    // sender. Two distinct paths:
    //   - Community voice: per-community pseudonym signing key (so
    //     receivers verify that the sender is a community member).
    //   - 1:1 DM call (W12-fix.A): user's identity Ed25519 secret. The
    //     packet carries the user's Ed25519 pubkey as `sender_key` and
    //     the receiver verifies against it; the call peer is the only
    //     legitimate sender for this transport so any other identity's
    //     packets get filtered downstream.
    //
    // Without this, `build_packet_data` returns "voice signing key not
    // installed" and EVERY outbound voice packet is dropped → caller
    // can talk into their mic but the receiver hears silence (the
    // primary "no audio" symptom in 1:1 calls).
    if let Some(cid) = community_id {
        if let Ok((_, signing_key)) = state_helpers::pseudonym_credentials(state, cid) {
            transport.set_signing_key(signing_key);
        } else {
            tracing::warn!(
                community = %cid,
                channel = %channel_id,
                "voice transport: pseudonym credentials unavailable, packets cannot be signed",
            );
        }
    } else {
        // 1:1 DM call. The user's local identity Ed25519 secret is the
        // signing key; the matching pubkey is what the receiver
        // already knows from the CallOffer/CallAccept handshake.
        let secret_bytes = state.identity_secret.lock().clone();
        match secret_bytes {
            Some(bytes) => {
                let signing_key = ed25519_dalek::SigningKey::from_bytes(&bytes);
                transport.set_signing_key(signing_key);
            }
            None => {
                tracing::warn!(
                    channel = %channel_id,
                    "voice transport: local identity secret unavailable, 1:1 call audio will be silent",
                );
            }
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
    transport: &std::sync::Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
    muted_flag: &Arc<AtomicBool>,
    deafened_flag: &Arc<AtomicBool>,
    member_names: std::collections::HashMap<String, String>,
) {
    // Set up voice packet receive channel
    let (voice_packet_tx, voice_packet_rx) = mpsc::channel(200);
    *state.voice_packet_tx.write() = Some(voice_packet_tx);

    // Speaker reference broadcast channel for AEC
    let (speaker_ref_tx, speaker_ref_rx) = broadcast::channel::<Vec<f32>>(50);

    // Pull community_id (for MEK encryption) and channel_id (for the
    // architecture §10.7 stage gate) from the active voice handle.
    let (voice_community_id, voice_channel_id) = {
        let ve = state.voice_engine.lock();
        ve.as_ref().map_or((None, String::new()), |h| {
            (h.community_id.clone(), h.channel_id.clone())
        })
    };

    // Spawn voice send loop
    let (send_shutdown_tx, send_shutdown_rx) = mpsc::channel::<()>(1);
    let send_handle = tokio::spawn(send_loop::run(send_loop::VoiceSendParams {
        capture_rx: bundle.capture_rx,
        transport: Arc::clone(transport),
        shutdown_rx: send_shutdown_rx,
        app: app.clone(),
        public_key: public_key.to_string(),
        noise_suppression: bundle.noise_suppression,
        echo_cancellation: bundle.echo_cancellation,
        muted_flag: Arc::clone(muted_flag),
        speaker_ref_rx,
        community_id: voice_community_id.clone(),
        channel_id: voice_channel_id,
        state: state.clone(),
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
        community_id: voice_community_id,
        state: state.clone(),
        member_names,
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

/// Load community member display names from SQLite for voice participant identification.
///
/// Returns pseudonym_key_hex → display_name mapping. Called once at voice session start.
async fn load_community_member_names(
    state: &crate::state::SharedState,
    community_id: Option<&str>,
) -> std::collections::HashMap<String, String> {
    use tauri::Manager as _;

    let Some(cid) = community_id else {
        return std::collections::HashMap::new();
    };
    let app_handle = state.app_handle.read().clone();
    let Some(ref ah) = app_handle else {
        return std::collections::HashMap::new();
    };
    let Some(pool) = ah.try_state::<crate::db::DbPool>() else {
        return std::collections::HashMap::new();
    };
    let cid_owned = cid.to_string();
    crate::db_helpers::db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, display_name FROM community_members WHERE community_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![cid_owned], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    })
    .await
    .unwrap_or_default()
}
