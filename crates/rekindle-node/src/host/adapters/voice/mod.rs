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
//! through [`HostVoice`]'s `MediaIngress` impl and
//! `rekindle_voice::ingress::handle_media_tag` ([`ingest_media`]).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use rekindle_codec::community::envelope::SignedEnvelope;
use rekindle_lifecycle::SessionScope;
use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeyStore;
use rekindle_types::subscription_events::VoiceScope;
use rekindle_voice::ingress::MediaIngress;
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

/// The live session: its scope, its engine and transport, the scopes its
/// loops run in, its local controls and the ledgers its loops read (the
/// desktop's `VoiceEngineHandle`, ported to the Host, plan E4.1 S5a2).
pub struct LiveVoice {
    pub scope: VoiceScope,
    /// Read by the send loop (muted) and the receive loop (deafened).
    pub muted: Arc<AtomicBool>,
    pub deafened: Arc<AtomicBool>,
    /// The transport's media roster (arrivals, feedback, allocator).
    pub media: Arc<MediaRoster>,
    /// Media-plane proof of life per sender.
    pub liveness: Arc<MediaLiveness>,
    /// The audio engine (devices, or none under `AudioIo::HEADLESS`).
    pub engine: Option<rekindle_voice::VoiceEngine>,
    /// The session's transport: its roster of peers and their routes.
    pub transport: Option<Arc<tokio::sync::Mutex<VoiceTransport>>>,
    /// The send and receive loops.
    pub loops: Option<Arc<SessionScope>>,
    /// The device monitor.
    pub monitor: Option<Arc<SessionScope>>,
    /// The MCU loop, while we host one.
    pub mcu: Option<Arc<SessionScope>>,
    /// The transport's per-peer egress drivers.
    pub egress: Option<Arc<SessionScope>>,
}

impl LiveVoice {
    /// A session on `scope` with fresh ledgers, no engine and no loops yet;
    /// its media roster reads `clock` (the Host's clock port).
    #[must_use]
    pub fn new(scope: VoiceScope, clock: Arc<dyn rekindle_utils::Clock>) -> Self {
        Self {
            scope,
            muted: Arc::default(),
            deafened: Arc::default(),
            media: Arc::new(MediaRoster::new(clock)),
            liveness: Arc::default(),
            engine: None,
            transport: None,
            loops: None,
            monitor: None,
            mcu: None,
            egress: None,
        }
    }

    /// Stop the engine's devices (the session stays).
    pub fn stop_devices(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.stop_capture();
            engine.stop_playback();
        }
    }
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
    drops: AtomicU64,
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
    pub(crate) quality: Mutex<rekindle_voice::VoiceQualityCache>,
}

impl HostVoice {
    #[must_use]
    pub fn new(clock: Arc<dyn rekindle_utils::Clock>, io: rekindle_voice::AudioIo) -> Self {
        Self {
            live: Mutex::new(None),
            drops: AtomicU64::new(0),
            clock,
            io,
            packet_tx: RwLock::new(None),
            packet_rx_staged: Mutex::new(None),
            report_tx: RwLock::new(None),
            sender_keys: ChannelSenderKeyStore::default(),
            quality: Mutex::default(),
        }
    }

    /// The Host's clock port.
    #[must_use]
    pub fn clock(&self) -> Arc<dyn rekindle_utils::Clock> {
        Arc::clone(&self.clock)
    }

    /// Why a new voice session cannot start now, in the Host's words, or
    /// `None` when idle. `community_name` names a community by id.
    pub fn start_refusal(&self, community_name: impl Fn(&str) -> String) -> Option<String> {
        let scope = self.session()?;
        Some(match &scope {
            VoiceScope::Community { community, .. } => leave_voice_in(&community_name(community)),
            VoiceScope::Dm { .. } | VoiceScope::Group { .. } => LEAVE_CALL_FIRST.to_owned(),
        })
    }

    /// Install `session` as the live one. The same scope again is already
    /// joined (`Ok`, nothing replaced); any other scope while one is live
    /// is refused with [`Self::start_refusal`]'s reason.
    ///
    /// # Errors
    /// The precedence reason when another session is live.
    pub fn begin(
        &self,
        session: LiveVoice,
        community_name: impl Fn(&str) -> String,
    ) -> Result<(), String> {
        let mut live = self.live.lock();
        if let Some(current) = live.as_ref() {
            if current.scope == session.scope {
                return Ok(());
            }
            drop(live);
            return Err(self
                .start_refusal(community_name)
                .unwrap_or_else(|| LEAVE_CALL_FIRST.to_owned()));
        }
        *live = Some(session);
        Ok(())
    }

    /// End the live session (leave and teardown): its devices stop, its
    /// loop channels go; its scope, if one ran.
    pub fn end(&self) -> Option<VoiceScope> {
        let ended = self.live.lock().take().map(|mut s| {
            s.stop_devices();
            s.scope
        });
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
        self.live
            .lock()
            .as_ref()
            .is_some_and(|s| s.engine.is_some())
    }

    /// Set the live session's mute; `false` when none runs.
    pub fn set_muted(&self, muted: bool) -> bool {
        self.live.lock().as_ref().is_some_and(|s| {
            s.muted.store(muted, Ordering::Relaxed);
            true
        })
    }

    /// Set the live session's deafen; `false` when none runs.
    pub fn set_deafened(&self, deafened: bool) -> bool {
        self.live.lock().as_ref().is_some_and(|s| {
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
        self.live.lock().as_ref().and_then(|s| s.transport.clone())
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
        let mut peers = match transport {
            Some(t) => t.lock().await.peer_keys(),
            None => Vec::new(),
        };
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
        self.drops.load(Ordering::Relaxed)
    }

    /// Count one dropped voice packet.
    pub fn note_drop(&self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl MediaIngress for HostVoice {
    fn media(&self) -> Option<Arc<MediaRoster>> {
        self.live.lock().as_ref().map(|s| Arc::clone(&s.media))
    }

    fn note_media_live(&self, sender: &str) {
        if let Some(s) = self.live.lock().as_ref() {
            s.liveness.note(sender, self.clock.now_ms());
        }
    }

    fn voice_packet_tx(&self) -> Option<mpsc::Sender<VoicePacket>> {
        self.packet_tx.read().clone()
    }

    fn receiver_report_tx(&self) -> Option<mpsc::Sender<VoiceReceiverReport>> {
        self.report_tx.read().clone()
    }

    fn note_voice_drop(&self) {
        self.note_drop();
    }

    fn queue_media_envelope(&self, signed: SignedEnvelope) {
        // The video-plane envelope's gossip ingress arm is the voice
        // signaling's (plan E4.1 S5a2); until it lands it is dropped here.
        tracing::debug!(sender = %signed.sender_pseudonym,
            "media envelope: the Host's voice signaling arms land in S5a2");
    }

    /// A verified report, accepted only from a peer on our voice roster
    /// about its own reception (r6 R-BW6, plans E4.3.2, E4.3.3).
    fn on_transport_feedback(&self, feedback: &TransportFeedback, arrived: Instant) {
        let Some(media) = self.media() else {
            return;
        };
        let reporter = hex::encode(&feedback.reporter_key);
        let Some(split) = media.on_transport_feedback(&reporter, feedback) else {
            media.feedback_stats().note_unknown_peer();
            tracing::debug!(peer = %reporter,
                "transport feedback from a peer not on our roster — dropped");
            return;
        };
        media.feedback_stats().note_accepted(&reporter);
        // A roster peer reporting on our media is alive even when silent.
        self.note_media_live(&reporter);
        tracing::debug!(
            peer = %reporter,
            reported = feedback.arrivals.len(),
            estimate_kbps = split.estimate_bps / 1_000,
            audio_bps = split.audio_bps,
            video_bps = split.video_bps,
            feedback_age_ms = arrived.elapsed().as_millis(),
            "transport feedback"
        );
    }
}

/// A media datagram from the transport's media lane, `arrived` stamped on
/// the dispatch thread: handed to the live session through the ingress
/// policy, never awaited.
pub fn ingest_media(voice: &HostVoice, raw: &[u8], arrived: Instant) {
    if !rekindle_voice::ingress::handle_media_tag(voice, raw, arrived) {
        tracing::debug!(
            len = raw.len(),
            "media lane datagram with no media tag — dropped"
        );
    }
}

mod deps;
mod events;
mod frame_sender;
mod requests;
mod setup;

pub use deps::HostVoiceDeps;
pub use requests::{join, leave};

#[cfg(test)]
mod tests;
