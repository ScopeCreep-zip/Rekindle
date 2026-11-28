//! The Host's one voice engine state (plan E4.1 S5a1; lead ruling
//! 2026-11-26: one engine, the Host's, ADR 0015 and ADR 0010), beside the
//! channel and presence adapters.
//!
//! One live voice session per identity across a 1:1 call, a group call and
//! community voice. Starting a second while one is live is refused with
//! the Host's reason ([`LEAVE_CALL_FIRST`], [`leave_voice_in`]); nothing
//! ends the first session implicitly, and leave is always available.
//!
//! The media fast path (`InboundHandler::on_media`) reaches the session
//! through the session's ingress binding and
//! `rekindle_voice::ingress::handle_media_tag` ([`ingest_media`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_lifecycle::SessionScope;
use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeyStore;
use rekindle_types::subscription_events::VoiceScope;
use rekindle_voice::liveness::{MediaLiveness, MEDIA_LIVE_WINDOW_MS};
use rekindle_voice::receiver_report::VoiceReceiverReport;
use rekindle_voice::transport::roster::MediaRoster;
use rekindle_voice::transport::{VoicePacket, VoiceTransport};
use tokio::sync::mpsc;

/// The refusal while a 1:1 or group call is live (lead ruling 2026-11-26).
pub const LEAVE_CALL_FIRST: &str = "Leave the current call first.";

/// The refusal while community voice in `community_name` is live (lead
/// ruling 2026-11-26).
#[must_use]
pub fn leave_voice_in(community_name: &str) -> String {
    format!("Leave voice in {community_name} first.")
}

/// The direct call that owns the session and its media keys.
pub(super) struct DirectMedia {
    pub id: String,
    pub peer: String,
    pub keys: rekindle_voice::CallMediaKeys,
}

/// The live session, its engine, transport, controls and child scopes.
pub(crate) struct LiveVoice {
    pub scope: VoiceScope,
    /// Read by the send loop (muted) and the receive loop (deafened).
    pub muted: Arc<AtomicBool>,
    pub deafened: Arc<AtomicBool>,
    /// The transport's media roster (arrivals, feedback, allocator).
    pub media: Arc<MediaRoster>,
    /// Media-plane proof of life per sender.
    pub liveness: Arc<MediaLiveness>,
    /// The audio engine (devices, or none under `AudioIo::HEADLESS`).
    pub engine: rekindle_voice::VoiceEngine,
    /// The session's transport: its roster of peers and their routes.
    pub transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    pub session_state: Arc<rekindle_voice::session_state::VoiceSessionState>,
    /// The send and receive loops.
    pub loops: Option<Arc<SessionScope>>,
    /// The device monitor.
    pub monitor: Option<Arc<SessionScope>>,
    /// The MCU loop, while we host one.
    pub mcu: Option<Arc<SessionScope>>,
    /// The transport's per-peer egress drivers.
    pub egress: Option<Arc<SessionScope>>,
}

/// The live session's roster as the Host reads it (plan E4.1 S5a2;
/// `VoiceStatus` in S5b): the peers on its transport and the senders whose
/// media arrived within [`MEDIA_LIVE_WINDOW_MS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRoster {
    pub scope: VoiceScope,
    /// The peers on the session's transport roster (their voice identity).
    pub peers: Vec<String>,
    /// The senders with media live now.
    pub media_live: Vec<String>,
}

impl LiveRoster {
    /// How many peers the session sends to.
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

/// The Host's voice engine state: one per Host, at most one live session.
pub struct HostVoice {
    pub(crate) live: Mutex<Option<LiveVoice>>,
    drops: rekindle_voice::packet_drops::PacketDrops,
    pub(super) transitions: tokio::sync::Mutex<()>,
    pub(super) call: Mutex<Option<DirectMedia>>,
    clock: Arc<dyn rekindle_utils::Clock>,
    /// Where audio comes from and goes (`HostConfig::audio`).
    pub(crate) io: rekindle_voice::AudioIo,
    /// The receive loop's packet channel (staged before the loops when a
    /// call is accepted).
    pub(crate) packet_tx: RwLock<Option<mpsc::Sender<VoicePacket>>>,
    pub(crate) packet_rx_staged: Mutex<Option<mpsc::Receiver<VoicePacket>>>,
    /// The send loop's receiver-report channel.
    pub(crate) report_tx: RwLock<Option<mpsc::Sender<VoiceReceiverReport>>>,
    /// Our and our peers' channel media sender keys (plans C7.20, C7.29).
    pub(crate) sender_keys: ChannelSenderKeyStore,
    /// The last quality and receive stats, merged into one event.
    pub(super) quality: Mutex<events::VoiceQualityCache>,
    pub(super) video_caps: RwLock<Option<rekindle_types::video::MediaCapabilities>>,
}

impl HostVoice {
    #[must_use]
    pub fn new(clock: Arc<dyn rekindle_utils::Clock>, io: rekindle_voice::AudioIo) -> Self {
        Self {
            live: Mutex::new(None),
            drops: rekindle_voice::packet_drops::PacketDrops::default(),
            transitions: tokio::sync::Mutex::new(()),
            call: Mutex::new(None),
            clock,
            io,
            packet_tx: RwLock::new(None),
            packet_rx_staged: Mutex::new(None),
            report_tx: RwLock::new(None),
            sender_keys: ChannelSenderKeyStore::default(),
            quality: Mutex::default(),
            video_caps: RwLock::default(),
        }
    }

    /// The Host's clock port.
    #[must_use]
    pub fn clock(&self) -> Arc<dyn rekindle_utils::Clock> {
        Arc::clone(&self.clock)
    }

    pub(crate) fn reset_unlock(&self) {
        self.sender_keys.clear();
        self.video_caps.write().take();
        self.drops.reset();
    }

    /// End the live session (leave and teardown): its devices stop, its
    /// loop channels go; its scope, if one ran.
    pub fn end(&self) -> Option<VoiceScope> {
        let ended = self.live.lock().take().map(|mut s| {
            s.engine.stop_capture();
            s.engine.stop_playback();
            s.scope
        });
        self.call.lock().take();
        *self.quality.lock() = events::VoiceQualityCache::default();
        *self.packet_tx.write() = None;
        *self.packet_rx_staged.lock() = None;
        *self.report_tx.write() = None;
        ended
    }

    /// The live session's scope.
    #[must_use]
    pub fn session(&self) -> Option<VoiceScope> {
        self.live.lock().as_ref().map(|s| s.scope.clone())
    }

    /// Whether the live session's engine is up.
    #[must_use]
    pub fn engine_present(&self) -> bool {
        self.live.lock().is_some()
    }

    /// Set the live session's mute; `false` when none runs.
    pub fn set_muted(&self, muted: bool) -> bool {
        self.live.lock().as_mut().is_some_and(|s| {
            s.engine.set_muted(muted);
            s.muted.store(muted, Ordering::Relaxed);
            true
        })
    }

    /// Set the live session's deafen; `false` when none runs.
    pub fn set_deafened(&self, deafened: bool) -> bool {
        self.live.lock().as_mut().is_some_and(|s| {
            s.engine.set_deafened(deafened);
            s.deafened.store(deafened, Ordering::Relaxed);
            true
        })
    }

    /// The live session's (muted, deafened), or `None`.
    #[must_use]
    pub fn controls(&self) -> Option<(bool, bool)> {
        self.live.lock().as_ref().map(|s| {
            (
                s.muted.load(Ordering::Relaxed),
                s.deafened.load(Ordering::Relaxed),
            )
        })
    }

    /// The live session's transport, while its engine is up.
    #[must_use]
    pub fn transport(&self) -> Option<Arc<tokio::sync::Mutex<VoiceTransport>>> {
        self.live.lock().as_ref().map(|s| s.transport.clone())
    }

    /// The live session's roster and media-live senders, or `None` with no
    /// session (the one read `VoiceStatus` uses, plan E4.1 S5b).
    pub async fn roster(&self) -> Option<LiveRoster> {
        let (scope, transport, liveness) = {
            let live = self.live.lock();
            let s = live.as_ref()?;
            (
                s.scope.clone(),
                s.transport.clone(),
                Arc::clone(&s.liveness),
            )
        };
        let mut peers = transport.lock().await.peer_keys();
        peers.sort();
        let mut media_live: Vec<String> = liveness
            .live_within(MEDIA_LIVE_WINDOW_MS, self.clock.now_ms())
            .into_iter()
            .collect();
        media_live.sort();
        Some(LiveRoster {
            scope,
            peers,
            media_live,
        })
    }

    /// Inbound voice packets dropped since the Host started.
    #[must_use]
    pub fn voice_drops(&self) -> u64 {
        self.drops.total_drops()
    }

    /// Count one dropped voice packet.
    pub fn note_drop(&self) {
        self.drops.record_receive();
    }
}

mod access;
mod carrier;
mod controls;
mod deps;
mod events;
mod frame_sender;
mod ingress;
mod io;
mod presence;
mod requests;
mod route_heal;
mod setup;
mod signaling;
mod signaling_events;
mod video;

pub(crate) use carrier::install_carrier;
pub(crate) use controls::{set_deafen, set_mute};
pub use deps::HostVoiceDeps;
pub(crate) use ingress::ingest_media;
pub use presence::HostVoicePresence;
pub(crate) use requests::{join, leave, leave_community, spawn_telemetry, start_refusal, stop};
pub(crate) use route_heal::spawn_route_heal;
pub(crate) use signaling::{answer_media_key_call, handle_signaling};
