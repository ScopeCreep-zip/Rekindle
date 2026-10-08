//! Phase 14.r split — voice session bring-up + teardown helpers.
//!
//! Contains the lifecycle helpers relocated from the deleted
//! `services/voice/session.rs` plus body-extractions for the big
//! trait methods (`init_voice_session`, `take_shutdown_handles`,
//! `spawn_voice_loops`). Each is a free fn taking explicit
//! references so the deps_impl method bodies stay short.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rekindle_voice::{
    VoiceError, VoiceLoopScopes, VoiceSessionDeps, VoiceSessionStartup, VoiceShutdownOpts,
};

use super::VoiceAdapter;
use crate::state::{AppState, VoiceEngineHandle};
use crate::state_helpers;
use rekindle_db::Db;

/// Channels and config needed to spawn the voice loops. Internal to
/// `spawn_voice_loops_impl`.
struct LoopBundle {
    capture_rx: Option<tokio::sync::mpsc::Receiver<Vec<f32>>>,
    playback_tx: Option<tokio::sync::mpsc::Sender<Vec<f32>>>,
    noise_suppression: bool,
    echo_cancellation: bool,
    /// Config-preset jitter base (ms) — the floor the adaptive receive
    /// buffer starts at (replaces the old hardcoded 200 ms).
    jitter_base_ms: u32,
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
        input_channels: prefs.input_channels.clone(),
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
        media: Arc::default(),
        loops: None,
        monitor: None,
        mcu: None,
        channel_id: channel_id.to_string(),
        community_id: community_id.map(String::from),
        muted_flag: Arc::clone(&muted_flag),
        deafened_flag: Arc::clone(&deafened_flag),
        media_liveness: Arc::new(rekindle_voice::liveness::MediaLiveness::default()),
    });

    // Now that the engine is on state, start the cpal devices. Device
    // bring-up is NON-FATAL: a member must be able to enter a voice channel
    // even when their mic or speakers can't open — listen-only when capture
    // fails, present-only when both fail — the way Discord and Mumble behave.
    // On Linux especially, cpal's ALSA backend can error or time out opening a
    // device held by PipeWire/PulseAudio. We log the failure but keep the
    // engine installed so the session still comes up; the send/receive loops
    // already tolerate an absent capture_rx / playback_tx. The engine now
    // corresponds to a real (possibly degraded) session, so leave/teardown
    // clears it normally and check_not_in_call stays correct.
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            if let Err(e) = handle.engine.start_capture() {
                tracing::warn!(
                    error = %e,
                    "voice capture device unavailable — joining without mic (listen-only)"
                );
            }
            if let Err(e) = handle.engine.start_playback() {
                tracing::warn!(
                    error = %e,
                    "voice playback device unavailable — joining without speaker output"
                );
            }
        }
    }

    // Build the real transport with full signing-key + AEAD wiring
    // (Veilid routing context init, ed25519 signing key for
    // packet signatures, call_key for 1:1 AEAD). The sender_key we
    // stamp on packets must be our voice self-identity (the community
    // pseudonym, or owner key for 1:1) so it matches the signing key
    // installed below and remote peers can verify our signature.
    let self_voice_id = state_helpers::voice_self_identity(state, community_id);
    if self_voice_id.is_empty() {
        return Err(VoiceError::IdentityNotLoaded);
    }
    let transport = create_transport_impl(
        state,
        &self_voice_id,
        channel_id,
        community_id,
        peer_route_blob,
    );
    let media = transport.media();
    let shared_transport = Arc::new(tokio::sync::Mutex::new(transport));

    // Install the real transport on the handle (overwrite the placeholder).
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.transport = Arc::clone(&shared_transport);
            handle.media = media;
        }
    }

    Ok(VoiceSessionStartup {
        muted_flag,
        deafened_flag,
        transport: shared_transport,
    })
}

/// Body of `VoiceSessionDeps::take_loop_scopes`: takes the scopes of
/// whatever loops the opts request off the engine, for the crate-side
/// teardown to shut down.
pub(super) fn take_loop_scopes_impl(state: &AppState, opts: VoiceShutdownOpts) -> VoiceLoopScopes {
    let mut ve = state.voice_engine.lock();
    let Some(ref mut handle) = *ve else {
        return VoiceLoopScopes {
            loops: None,
            monitor: None,
            mcu: None,
        };
    };
    VoiceLoopScopes {
        loops: if opts.stop_loops {
            handle.loops.take()
        } else {
            None
        },
        mcu: if opts.stop_loops {
            handle.mcu.take()
        } else {
            None
        },
        monitor: if opts.stop_monitor {
            handle.monitor.take()
        } else {
            None
        },
    }
}

/// Build the shared VoiceTransport with full Veilid api init +
/// signing key + (for 1:1) AEAD call_key install. Relocated verbatim
/// from the deleted `services::voice::session::create_transport`.
fn create_transport_impl(
    state: &Arc<AppState>,
    self_voice_id: &str,
    channel_id: &str,
    community_id: Option<&str>,
    resolved_peer_route: Option<&[u8]>,
) -> rekindle_voice::transport::VoiceTransport {
    let mut transport = rekindle_voice::transport::VoiceTransport::new(channel_id.to_string());
    let api = state_helpers::veilid_api(state);
    let sender_key = hex::decode(self_voice_id).unwrap_or_default();

    // Routes resolve through the process's one importer (plan C7.6).
    let sender = match (api, state_helpers::route_imports(state)) {
        (Some(api), Ok(imports)) => {
            match super::frame_sender::VeilidVoiceFrameSender::new(&api, imports) {
                Ok(sender) => Some(Arc::new(sender) as Arc<dyn rekindle_voice::VoiceFrameSender>),
                Err(e) => {
                    tracing::warn!(error = %e, "voice frame sender unavailable");
                    None
                }
            }
        }
        _ => None,
    };
    if let Some(sender) = sender {
        // Each roster peer's egress driver runs here, ending with the
        // login at the latest; leaving the call stops them one by one.
        let egress = state_helpers::login_scope_or_closed(state).child("voice media egress");
        if community_id.is_some() {
            transport.init(sender, sender_key, egress);
        } else if let Some(blob) = resolved_peer_route {
            // For a DM call the channel id *is* the peer's public key,
            // so the roster is keyed by the peer's real identity.
            transport.connect(sender, blob, sender_key, channel_id, egress);
        } else {
            transport.init(sender, sender_key, egress);
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
    }

    transport
}

fn take_channels_and_config(state: &AppState) -> Result<LoopBundle, String> {
    let mut ve = state.voice_engine.lock();
    let handle = ve.as_mut().ok_or("no active voice engine")?;
    let ns = handle.engine.config().noise_suppression;
    let ec = handle.engine.config().echo_cancellation;
    let jitter_base_ms = handle.engine.config().jitter_buffer_ms;
    Ok(LoopBundle {
        capture_rx: handle.engine.take_capture_rx(),
        playback_tx: handle.engine.take_playback_tx(),
        noise_suppression: ns,
        echo_cancellation: ec,
        jitter_base_ms,
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
    pool: &Db,
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

    // The liveness ledger is shared: both loops note into the SAME
    // Arc the engine handle owns, and the signaling adapter reads it
    // for the presence reconcile's media veto.
    let (voice_community_id, voice_channel_id, media_liveness, playback_depth_ms, media) = {
        let ve = state.voice_engine.lock();
        ve.as_ref().map_or_else(
            || {
                (
                    None,
                    String::new(),
                    Arc::new(rekindle_voice::liveness::MediaLiveness::default()),
                    Arc::default(),
                    Arc::default(),
                )
            },
            |h| {
                (
                    h.community_id.clone(),
                    h.channel_id.clone(),
                    Arc::clone(&h.media_liveness),
                    h.engine.playback_depth(),
                    Arc::clone(&h.media),
                )
            },
        )
    };
    let follower_scope = voice_community_id
        .clone()
        .map(|c| (c, voice_channel_id.clone()));
    let recv_allocator = Arc::clone(media.allocator());
    let recv_arrivals = Arc::clone(media.arrivals());
    let recv_quality = Arc::clone(media.quality());
    let recv_feedback_stats = Arc::clone(media.feedback_stats());

    // Build a per-loop adapter Arc for the crate-side loop deps.
    let adapter_for_send: Arc<dyn VoiceSessionDeps> =
        VoiceAdapter::new(state.clone(), app.clone(), pool.clone());
    let adapter_for_recv: Arc<dyn VoiceSessionDeps> =
        VoiceAdapter::new(state.clone(), app.clone(), pool.clone());

    // Receiver reports must be signed with the same identity the send
    // side signs packets with (see `build_transport`), or peers reject
    // them: the community pseudonym for channel voice, the account key
    // for a 1:1 call. `None` disables reporting rather than emitting
    // reports nobody will accept.
    let report_signing_key = if let Some(cid) = voice_community_id.as_deref() {
        state_helpers::pseudonym_credentials(state, cid)
            .ok()
            .map(|(_, signing_key)| signing_key)
    } else {
        let secret = *state.identity_secret.lock();
        secret.map(|bytes| ed25519_dalek::SigningKey::from_bytes(&bytes))
    };
    if report_signing_key.is_none() {
        tracing::warn!(
            "voice: no signing identity for receiver reports — the send side will fall back to \
             local send-failure counts for quality"
        );
    }

    // The return path: dispatch loop → send loop. Depth 32 is ~2.5
    // minutes of reports from one peer; a full channel means the send
    // loop is wedged, and dropping is right because the next report
    // supersedes the one dropped.
    let (report_tx, report_rx) = mpsc::channel(32);
    *state.voice_report_tx.write() = Some(report_tx);

    // The loops run in a child of the login scope: they end with the
    // voice session, and at the latest with the login (plan C4).
    let login = state_helpers::login_scope(state)
        .ok_or_else(|| "voice needs a login session".to_string())?;
    let loops = login.child("voice loops");
    let send_params = {
        let transport = Arc::clone(transport);
        let muted_flag = Arc::clone(muted_flag);
        let media_liveness = Arc::clone(&media_liveness);
        let our_pseudonym = voice_community_id
            .as_deref()
            .and_then(|cid| crate::state_helpers::my_pseudonym_key(state, cid));
        let community_id = voice_community_id.clone();
        let channel_id = voice_channel_id.clone();
        let public_key = public_key.to_string();
        move |stop| rekindle_voice::send_loop::VoiceSendParams {
            capture_rx: bundle.capture_rx,
            transport,
            stop,
            deps: adapter_for_send,
            public_key,
            noise_suppression: bundle.noise_suppression,
            echo_cancellation: bundle.echo_cancellation,
            muted_flag,
            speaker_ref_rx,
            community_id,
            channel_id,
            our_pseudonym,
            report_rx,
            media_liveness,
        }
    };
    loops
        .spawn_with_token("voice send loop", |stop| {
            rekindle_voice::send_loop::run(send_params(stop))
        })
        .map_err(|e| e.to_string())?;

    let recv_params = move |stop| rekindle_voice::receive_loop::VoiceReceiveParams {
        packet_rx: voice_packet_rx,
        playback_tx: bundle.playback_tx,
        stop,
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
        jitter_base_ms: bundle.jitter_base_ms,
        report_signing_key,
        media_liveness,
        playback_depth_ms,
        arrivals: recv_arrivals,
        allocator: recv_allocator,
        quality: recv_quality,
        feedback_stats: recv_feedback_stats,
    };
    loops
        .spawn_with_token("voice receive loop", |stop| {
            rekindle_voice::receive_loop::run(recv_params(stop))
        })
        .map_err(|e| e.to_string())?;

    let device_error_rx = {
        let mut ve = state.voice_engine.lock();
        ve.as_mut().and_then(|h| {
            h.engine
                .take_device_error_rx()
                .or_else(|| Some(h.engine.refresh_device_error_channels()))
        })
    };
    let monitor = login.child("voice device monitor");
    if let Some(error_rx) = device_error_rx {
        let adapter_for_monitor: Arc<dyn VoiceSessionDeps> =
            VoiceAdapter::new(state.clone(), app.clone(), pool.clone());
        monitor
            .spawn_with_token("voice device monitor", |stop| {
                rekindle_voice::session::device_monitor::run(
                    rekindle_voice::session::device_monitor::DeviceMonitorParams {
                        device_error_rx: error_rx,
                        stop,
                        deps: adapter_for_monitor,
                    },
                )
            })
            .map_err(|e| e.to_string())?;
    }

    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.loops = Some(Arc::clone(&loops));
            handle.monitor = Some(monitor);
        }
    }

    // Community video follows the allocator: encoder targets and the
    // keyframes a route asks for after dropping or pausing video (plan
    // E4.3.3). DM video is 1:1 over its own path.
    if let Some((community_id, channel_id)) = follower_scope {
        let follower = super::video_allocation::VideoAllocationFollower {
            state: Arc::clone(state),
            app: app.clone(),
            allocator: Arc::clone(media.allocator()),
            community_id,
            channel_id,
        };
        loops
            .spawn_with_token("video allocation", |stop| follower.run(stop))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Phase B — seed the per-call video session state and force-emit the
/// current `SessionVideoConfig`, so a late-mounting frontend gets the
/// policy event without waiting for a membership delta.
///
/// Community sessions only: DM video shape is policed by a different
/// code path and carries no `SessionVideoConfig`.
pub(super) fn seed_community_media_session(
    adapter: &VoiceAdapter,
    community_id: &str,
    channel_id: &str,
) {
    // Seed the media-ready gate BEFORE the config emit below, so its
    // `session_config_emitted` hook lands on a slot whose other inputs
    // already reflect reality. No key is acquired here: our media key is
    // our own, and each participant sends us theirs as it adds us (plan
    // C7.20).
    let caps_reported =
        crate::services::community::video_session::reported_local_caps(&adapter.state).is_some();
    crate::services::community::media_ready_runtime::update_media_ready(
        &adapter.state,
        community_id,
        channel_id,
        |i| {
            i.local_caps_reported = caps_reported;
        },
    );
    if let Err(e) = crate::services::community::video_session::on_local_joined(
        &adapter.state,
        community_id,
        channel_id,
    ) {
        tracing::warn!(error = %e, "video_session::on_local_joined failed");
    }
}
