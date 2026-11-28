//! Voice session bring-up and teardown on the Host (the desktop's
//! `voice_adapter::session_setup`, ported faithfully, plan E4.1 S5a2): the
//! engine on `HostConfig::audio`, the transport over the media-class frame
//! sender, the send/receive loops and the device monitor in children of the
//! unlock scope.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rekindle_voice::{
    AudioPrefs, VoiceError, VoiceLoopScopes, VoiceSessionDeps, VoiceSessionStartup,
    VoiceShutdownOpts,
};

use super::deps::HostVoiceDeps;
use super::{events, LiveVoice};

type SharedTransport = Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>;

/// Install the engine and the transport for a session on `channel_id`.
pub(super) fn init_voice_session(
    deps: &HostVoiceDeps,
    prefs: &AudioPrefs,
    channel_id: &str,
    community_id: Option<&str>,
    peer_route_blob: Option<&[u8]>,
) -> Result<VoiceSessionStartup, VoiceError> {
    let voice = deps.voice();
    let clock = voice.clock();
    let self_voice_id = deps.voice_self_identity(community_id);
    if self_voice_id.is_empty() {
        return Err(VoiceError::IdentityNotLoaded);
    }
    let (start_muted, start_deafened) = deps.initial_controls();
    let mut engine = rekindle_voice::VoiceEngine::open(prefs, clock)?;
    engine.set_muted(start_muted);
    engine.set_deafened(start_deafened);
    let (transport, egress) = create_transport(
        deps,
        &self_voice_id,
        channel_id,
        community_id,
        peer_route_blob,
    )?;
    let muted = Arc::new(AtomicBool::new(start_muted));
    let deafened = Arc::new(AtomicBool::new(start_deafened));
    let session_state = transport.session_state();
    let media = transport.media();
    let transport = Arc::new(tokio::sync::Mutex::new(transport));
    *voice.live.lock() = Some(LiveVoice {
        scope: events::scope_of(channel_id, community_id),
        muted: Arc::clone(&muted),
        deafened: Arc::clone(&deafened),
        media,
        liveness: Arc::default(),
        engine,
        transport: Arc::clone(&transport),
        session_state,
        loops: None,
        monitor: None,
        mcu: None,
        egress: Some(egress),
    });
    Ok(VoiceSessionStartup {
        muted_flag: muted,
        deafened_flag: deafened,
        transport,
    })
}

/// The session's transport: our voice identity signs its packets, its
/// frames leave on the media-class send.
fn create_transport(
    deps: &HostVoiceDeps,
    self_voice_id: &str,
    channel_id: &str,
    community_id: Option<&str>,
    peer_route: Option<&[u8]>,
) -> Result<
    (
        rekindle_voice::transport::VoiceTransport,
        Arc<rekindle_lifecycle::SessionScope>,
    ),
    VoiceError,
> {
    let voice = deps.voice();
    let mut transport =
        rekindle_voice::transport::VoiceTransport::new(channel_id.to_string(), voice.clock());
    let node = deps
        .ctx()
        .transport
        .read()
        .clone()
        .ok_or_else(|| VoiceError::Transport("the transport has not started".into()))?;
    let sender: Arc<dyn rekindle_voice::VoiceFrameSender> =
        Arc::new(super::frame_sender::HostFrameSender(node.caller()));
    let sender_key = hex::decode(self_voice_id).map_err(|_| VoiceError::IdentityNotLoaded)?;
    let signing_key = deps
        .signing_key(community_id)
        .ok_or(VoiceError::IdentityNotLoaded)?;
    transport.set_signing_key(signing_key);
    // Each roster peer's egress driver runs here; teardown shuts it down.
    let egress = deps
        .ctx()
        .unlock_scope_or_closed()
        .child("voice media egress");
    match (community_id, peer_route) {
        (None, Some(blob)) => {
            // A 1:1 call's channel id is the peer's key.
            transport.connect(sender, blob, sender_key, channel_id, Arc::clone(&egress));
        }
        (Some(_), _) => transport.init(sender, sender_key, Arc::clone(&egress)),
        (None, None) => return Err(VoiceError::NotConnected),
    }
    Ok((transport, egress))
}

/// The engine's capture/playback channels and tuning, taken for the loops.
struct LoopBundle {
    capture_rx: Option<tokio::sync::mpsc::Receiver<Vec<f32>>>,
    playback: Option<rekindle_voice::playback::PlaybackOutput>,
    noise_suppression: bool,
    echo_cancellation: bool,
    jitter_base_ms: u32,
    playback_depth_ms: Arc<std::sync::atomic::AtomicU32>,
    device_error_rx: Option<tokio::sync::mpsc::Receiver<String>>,
}

fn take_bundle(deps: &HostVoiceDeps) -> Result<(LoopBundle, LiveParts), VoiceError> {
    let voice = deps.voice();
    let mut live = voice.live.lock();
    let session = live
        .as_mut()
        .ok_or_else(|| VoiceError::Session("no active voice engine".into()))?;
    let parts = LiveParts {
        scope: session.scope.clone(),
        liveness: Arc::clone(&session.liveness),
        media: Arc::clone(&session.media),
    };
    let engine = &mut session.engine;
    let config = engine.config();
    let bundle = LoopBundle {
        noise_suppression: config.noise_suppression,
        echo_cancellation: config.echo_cancellation,
        jitter_base_ms: config.jitter_buffer_ms,
        capture_rx: engine.take_capture_rx(),
        playback: engine.take_playback_output(),
        playback_depth_ms: engine.playback_depth(),
        device_error_rx: Some(
            engine
                .take_device_error_rx()
                .unwrap_or_else(|| engine.refresh_device_error_channels()),
        ),
    };
    Ok((bundle, parts))
}

struct LiveParts {
    scope: rekindle_types::subscription_events::VoiceScope,
    liveness: Arc<rekindle_voice::liveness::MediaLiveness>,
    media: Arc<rekindle_voice::transport::roster::MediaRoster>,
}

/// Spawn the send and receive loops and the device monitor.
pub(super) fn spawn_voice_loops(
    deps: &HostVoiceDeps,
    public_key: &str,
    transport: &SharedTransport,
    muted_flag: &Arc<AtomicBool>,
    deafened_flag: &Arc<AtomicBool>,
) -> Result<(), VoiceError> {
    use rekindle_types::subscription_events::VoiceScope;
    use tokio::sync::{broadcast, mpsc};

    let (bundle, parts) = take_bundle(deps)?;
    let voice = deps.voice();
    let voice_packet_rx = voice.packet_rx_staged.lock().take().unwrap_or_else(|| {
        let (tx, rx) = mpsc::channel(200);
        *voice.packet_tx.write() = Some(tx);
        rx
    });
    let (community_id, channel_id) = match &parts.scope {
        VoiceScope::Community { community, channel } => (Some(community.clone()), channel.clone()),
        VoiceScope::Dm { peer_key } => (None, peer_key.clone()),
    };
    let (speaker_ref_tx, speaker_ref_rx) = broadcast::channel::<Vec<f32>>(50);
    // Reports are signed with the identity our packets are signed with.
    let report_signing_key = deps.signing_key(community_id.as_deref());
    // Depth 32: a full channel means a wedged send loop; the next report
    // supersedes a dropped one.
    let (report_tx, report_rx) = mpsc::channel(32);
    *voice.report_tx.write() = Some(report_tx);

    let unlock = deps.ctx().unlock_scope_or_closed();
    let loops = unlock.child("voice loops");
    let deps_send: Arc<dyn VoiceSessionDeps> = Arc::new(deps.clone());
    let deps_recv: Arc<dyn VoiceSessionDeps> = Arc::new(deps.clone());
    let send = rekindle_voice::send_loop::VoiceSendParams {
        capture_rx: bundle.capture_rx,
        transport: Arc::clone(transport),
        stop: tokio_util::sync::CancellationToken::new(),
        deps: deps_send,
        public_key: public_key.to_string(),
        noise_suppression: bundle.noise_suppression,
        echo_cancellation: bundle.echo_cancellation,
        muted_flag: Arc::clone(muted_flag),
        speaker_ref_rx,
        community_id: community_id.clone(),
        channel_id: channel_id.clone(),
        our_pseudonym: community_id.as_deref().map(|c| deps.ctx().my_pseudonym(c)),
        report_rx,
        media_liveness: Arc::clone(&parts.liveness),
    };
    loops
        .spawn_with_token("voice send loop", |stop| {
            rekindle_voice::send_loop::run(rekindle_voice::send_loop::VoiceSendParams {
                stop,
                ..send
            })
        })
        .map_err(|e| VoiceError::Session(e.to_string()))?;
    let recv = rekindle_voice::receive_loop::VoiceReceiveParams {
        packet_rx: voice_packet_rx,
        playback: bundle.playback,
        stop: tokio_util::sync::CancellationToken::new(),
        deps: deps_recv,
        our_public_key: public_key.to_string(),
        deafened_flag: Arc::clone(deafened_flag),
        speaker_ref_tx,
        community_id,
        channel_id: (!channel_id.is_empty()).then_some(channel_id),
        jitter_base_ms: bundle.jitter_base_ms,
        report_signing_key,
        media_liveness: parts.liveness,
        playback_depth_ms: bundle.playback_depth_ms,
        arrivals: Arc::clone(parts.media.arrivals()),
        allocator: Arc::clone(parts.media.allocator()),
        quality: Arc::clone(parts.media.quality()),
        feedback_stats: Arc::clone(parts.media.feedback_stats()),
    };
    loops
        .spawn_with_token("voice receive loop", |stop| {
            rekindle_voice::receive_loop::run(rekindle_voice::receive_loop::VoiceReceiveParams {
                stop,
                ..recv
            })
        })
        .map_err(|e| VoiceError::Session(e.to_string()))?;

    let monitor = unlock.child("voice device monitor");
    if let Some(error_rx) = bundle.device_error_rx {
        let deps_monitor: Arc<dyn VoiceSessionDeps> = Arc::new(deps.clone());
        monitor
            .spawn_with_token("voice device monitor", |stop| {
                rekindle_voice::session::device_monitor::run(
                    rekindle_voice::session::device_monitor::DeviceMonitorParams {
                        device_error_rx: error_rx,
                        stop,
                        deps: deps_monitor,
                    },
                )
            })
            .map_err(|e| VoiceError::Session(e.to_string()))?;
    }
    if let Some(session) = voice.live.lock().as_mut() {
        session.loops = Some(loops);
        session.monitor = Some(monitor);
    }
    Ok(())
}

/// The scopes a shutdown with `opts` stops, taken from the live session.
pub(super) fn take_loop_scopes(deps: &HostVoiceDeps, opts: VoiceShutdownOpts) -> VoiceLoopScopes {
    let voice = deps.voice();
    let mut live = voice.live.lock();
    let Some(s) = live.as_mut() else {
        return VoiceLoopScopes {
            loops: None,
            monitor: None,
            mcu: None,
            egress: None,
        };
    };
    VoiceLoopScopes {
        loops: if opts.stop_loops {
            s.loops.take()
        } else {
            None
        },
        mcu: if opts.stop_loops { s.mcu.take() } else { None },
        monitor: if opts.stop_monitor {
            s.monitor.take()
        } else {
            None
        },
        // The transport's drivers end with the session, not a restart.
        egress: if opts.stop_devices {
            s.egress.take()
        } else {
            None
        },
    }
}
