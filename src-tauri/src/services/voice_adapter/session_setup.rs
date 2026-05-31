//! Phase 14.r split — voice session bring-up + teardown helpers.
//!
//! Contains the lifecycle helpers relocated from the deleted
//! `services/voice/session.rs` plus body-extractions for the big
//! trait methods (`init_voice_session`, `take_shutdown_handles`,
//! `spawn_voice_loops`). Each is a free fn taking explicit
//! references so the deps_impl method bodies stay short.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rekindle_voice::{
    VoiceError, VoiceSessionDeps, VoiceSessionStartup, VoiceShutdownHandles, VoiceShutdownOpts,
};

use super::VoiceAdapter;
use crate::db::DbPool;
use crate::state::{AppState, VoiceEngineHandle};
use crate::state_helpers;

/// Channels and config needed to spawn the voice loops. Internal to
/// `spawn_voice_loops_impl`.
struct LoopBundle {
    capture_rx: Option<tokio::sync::mpsc::Receiver<Vec<f32>>>,
    playback_tx: Option<tokio::sync::mpsc::Sender<Vec<f32>>>,
    noise_suppression: bool,
    echo_cancellation: bool,
}

/// Body of `VoiceSessionDeps::init_voice_session` — extracted as a
/// free fn so deps_impl stays under the file-size cap. Builds the
/// VoiceEngine, installs the handle on AppState, starts cpal devices,
/// constructs the real VoiceTransport with full signing-key + AEAD
/// wiring, and returns the muted/deafened flags + transport for the
/// crate's session orchestrator to thread into the loops.
pub(super) fn init_voice_session_impl(
    state: &Arc<AppState>,
    prefs: &rekindle_voice::AudioPrefs,
    channel_id: &str,
    community_id: Option<&str>,
    peer_route_blob: Option<&[u8]>,
) -> Result<VoiceSessionStartup, VoiceError> {
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
        .map_err(|e| VoiceError::Session(format!("failed to create voice engine: {e}")))?;

    // First install the engine handle on AppState so
    // start_audio_devices + create_transport (both AppState-coupled)
    // can find it.
    *state.voice_engine.lock() = Some(VoiceEngineHandle {
        engine,
        // Placeholder transport — overwritten below once we've
        // built the real one with signing key + AEAD installed.
        transport: Arc::new(tokio::sync::Mutex::new(
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

    // Now that the engine is on state, start the cpal devices.
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle
                .engine
                .start_capture()
                .map_err(|e| VoiceError::Session(format!("start capture: {e}")))?;
            handle
                .engine
                .start_playback()
                .map_err(|e| VoiceError::Session(format!("start playback: {e}")))?;
        }
    }

    // Build the real transport with full signing-key + AEAD wiring
    // (Veilid routing context init, ed25519 signing key for
    // packet signatures, call_key for 1:1 AEAD).
    let owner_pubkey = state_helpers::current_owner_key(state)
        .map_err(|_| VoiceError::IdentityNotLoaded)?;
    let transport =
        create_transport_impl(state, &owner_pubkey, channel_id, community_id, peer_route_blob);
    let shared_transport = Arc::new(tokio::sync::Mutex::new(transport));

    // Install the real transport on the handle (overwrite the
    // placeholder).
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.transport = Arc::clone(&shared_transport);
        }
    }

    Ok(VoiceSessionStartup {
        muted_flag,
        deafened_flag,
        transport: shared_transport,
    })
}

/// Body of `VoiceSessionDeps::take_shutdown_handles` — extracted to
/// keep deps_impl readable. Drains the shutdown senders + join
/// handles for whatever loops the opts request; returns them so the
/// crate-side teardown can `.send(())` then `.await` each.
pub(super) fn take_shutdown_handles_impl(
    state: &AppState,
    opts: VoiceShutdownOpts,
) -> VoiceShutdownHandles {
    let mut ve = state.voice_engine.lock();
    if let Some(ref mut handle) = *ve {
        let (send_tx, send_h, recv_tx, recv_h, mcu_tx, mcu_h) = if opts.stop_loops {
            (
                handle.send_loop_shutdown.take(),
                handle.send_loop_handle.take(),
                handle.recv_loop_shutdown.take(),
                handle.recv_loop_handle.take(),
                handle.mcu_loop_shutdown.take(),
                handle.mcu_loop_handle.take(),
            )
        } else {
            (None, None, None, None, None, None)
        };
        let (monitor_tx, monitor_h) = if opts.stop_monitor {
            (
                handle.device_monitor_shutdown.take(),
                handle.device_monitor_handle.take(),
            )
        } else {
            (None, None)
        };
        VoiceShutdownHandles {
            send_loop_shutdown: send_tx,
            send_loop_handle: send_h,
            recv_loop_shutdown: recv_tx,
            recv_loop_handle: recv_h,
            monitor_shutdown: monitor_tx,
            monitor_handle: monitor_h,
            mcu_shutdown: mcu_tx,
            mcu_handle: mcu_h,
        }
    } else {
        VoiceShutdownHandles {
            send_loop_shutdown: None,
            send_loop_handle: None,
            recv_loop_shutdown: None,
            recv_loop_handle: None,
            monitor_shutdown: None,
            monitor_handle: None,
            mcu_shutdown: None,
            mcu_handle: None,
        }
    }
}

/// Build the shared VoiceTransport with full Veilid api init +
/// signing key + (for 1:1) AEAD call_key install. Relocated verbatim
/// from the deleted `services::voice::session::create_transport`.
fn create_transport_impl(
    state: &Arc<AppState>,
    public_key: &str,
    channel_id: &str,
    community_id: Option<&str>,
    resolved_peer_route: Option<&[u8]>,
) -> rekindle_voice::transport::VoiceTransport {
    let mut transport = rekindle_voice::transport::VoiceTransport::new(channel_id.to_string());
    let api = state_helpers::veilid_api(state);
    let sender_key = hex::decode(public_key).unwrap_or_default();

    if let Some(api) = api {
        if community_id.is_some() {
            transport.init(api, sender_key);
        } else if let Some(blob) = resolved_peer_route {
            if let Err(e) = transport.connect(api, blob, sender_key) {
                tracing::warn!(error = %e, channel = %channel_id,
                    "voice transport connect failed — audio only local");
            }
        } else {
            transport.init(api, sender_key);
        }
    }

    if let Some(cid) = community_id {
        if let Ok((_, signing_key)) = state_helpers::pseudonym_credentials(state, cid) {
            transport.set_signing_key(signing_key);
        } else {
            tracing::warn!(community = %cid, channel = %channel_id,
                "voice transport: pseudonym credentials unavailable, packets cannot be signed");
        }
    } else {
        let secret_bytes = *state.identity_secret.lock();
        if let Some(bytes) = secret_bytes {
            let signing_key = ed25519_dalek::SigningKey::from_bytes(&bytes);
            transport.set_signing_key(signing_key);
        } else {
            tracing::warn!(channel = %channel_id,
                "voice transport: local identity secret unavailable, 1:1 call audio will be silent");
        }
        let call_key_opt = state
            .active_calls
            .list_all()
            .into_iter()
            .find(|c| c.peer_pubkey == channel_id)
            .and_then(|c| c.call_key);
        if let Some(key) = call_key_opt {
            tracing::info!(channel = %channel_id, "1:1 voice transport: call_key installed (AEAD active)");
            transport.set_call_key(key);
        } else {
            tracing::warn!(channel = %channel_id,
                "1:1 voice transport: NO call_key on CallState — audio will fail AEAD on receiver");
        }
    }

    transport
}

fn take_channels_and_config(state: &AppState) -> Result<LoopBundle, String> {
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

/// Spawn send/receive/device-monitor loops + register handles on the
/// engine. Relocated from the deleted `services::voice::session::
/// spawn_loops` + the per-loop facades in `services::voice::{send_loop,
/// receive_loop, device_monitor}`. Each loop runs against a freshly
/// constructed `VoiceAdapter` (fresh `Arc<dyn VoiceSessionDeps>`).
pub(super) fn spawn_voice_loops_impl(
    state: &Arc<AppState>,
    app: &tauri::AppHandle,
    pool: &DbPool,
    public_key: &str,
    transport: &Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
    muted_flag: &Arc<AtomicBool>,
    deafened_flag: &Arc<AtomicBool>,
    member_names: HashMap<String, String>,
) -> Result<(), String> {
    use tokio::sync::{broadcast, mpsc};

    let bundle = take_channels_and_config(state)?;

    // W14.1 — use the pre-staged receiver if one is present.
    let voice_packet_rx = {
        let mut staged = state.voice_packet_rx_staged.lock();
        if let Some(rx) = staged.take() {
            tracing::info!("voice receive: using pre-staged channel from CallAccept handler");
            rx
        } else {
            let (tx, rx) = mpsc::channel(200);
            *state.voice_packet_tx.write() = Some(tx);
            rx
        }
    };

    let (speaker_ref_tx, speaker_ref_rx) = broadcast::channel::<Vec<f32>>(50);

    let (voice_community_id, voice_channel_id) = {
        let ve = state.voice_engine.lock();
        ve.as_ref().map_or((None, String::new()), |h| {
            (h.community_id.clone(), h.channel_id.clone())
        })
    };

    // Build a per-loop adapter Arc for the crate-side loop deps.
    let adapter_for_send: Arc<dyn VoiceSessionDeps> =
        VoiceAdapter::new(state.clone(), app.clone(), pool.clone());
    let adapter_for_recv: Arc<dyn VoiceSessionDeps> =
        VoiceAdapter::new(state.clone(), app.clone(), pool.clone());

    let (send_shutdown_tx, send_shutdown_rx) = mpsc::channel::<()>(1);
    let send_handle = tokio::spawn(rekindle_voice::send_loop::run(
        rekindle_voice::send_loop::VoiceSendParams {
            capture_rx: bundle.capture_rx,
            transport: Arc::clone(transport),
            shutdown_rx: send_shutdown_rx,
            deps: adapter_for_send,
            public_key: public_key.to_string(),
            noise_suppression: bundle.noise_suppression,
            echo_cancellation: bundle.echo_cancellation,
            muted_flag: Arc::clone(muted_flag),
            speaker_ref_rx,
            community_id: voice_community_id.clone(),
            channel_id: voice_channel_id.clone(),
            our_pseudonym: voice_community_id.as_deref().and_then(|cid| {
                state
                    .communities
                    .read()
                    .get(cid)
                    .and_then(|c| c.my_pseudonym_key.clone())
            }),
        },
    ));

    let (recv_shutdown_tx, recv_shutdown_rx) = mpsc::channel::<()>(1);
    let recv_handle = tokio::spawn(rekindle_voice::receive_loop::run(
        rekindle_voice::receive_loop::VoiceReceiveParams {
            packet_rx: voice_packet_rx,
            playback_tx: bundle.playback_tx,
            shutdown_rx: recv_shutdown_rx,
            deps: adapter_for_recv,
            our_public_key: public_key.to_string(),
            deafened_flag: Arc::clone(deafened_flag),
            speaker_ref_tx,
            community_id: voice_community_id,
            channel_id: if voice_channel_id.is_empty() {
                None
            } else {
                Some(voice_channel_id)
            },
            member_names,
        },
    ));

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
        let adapter_for_monitor: Arc<dyn VoiceSessionDeps> =
            VoiceAdapter::new(state.clone(), app.clone(), pool.clone());
        tokio::spawn(rekindle_voice::session::device_monitor::run(
            rekindle_voice::session::device_monitor::DeviceMonitorParams {
                device_error_rx: error_rx,
                shutdown_rx: monitor_shutdown_rx,
                deps: adapter_for_monitor,
            },
        ))
    });

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
