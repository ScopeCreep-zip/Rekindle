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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use rekindle_codec::community::envelope::SignedEnvelope;
use rekindle_types::subscription_events::VoiceScope;
use rekindle_voice::ingress::MediaIngress;
use rekindle_voice::liveness::MediaLiveness;
use rekindle_voice::receiver_report::VoiceReceiverReport;
use rekindle_voice::transport::roster::MediaRoster;
use rekindle_voice::transport::VoicePacket;
use tokio::sync::mpsc;

/// The refusal while a 1:1 or group call is live (lead ruling 2026-11-26).
pub const LEAVE_CALL_FIRST: &str = "Leave the current call first.";

/// The refusal while community voice in `community_name` is live (lead
/// ruling 2026-11-26).
#[must_use]
pub fn leave_voice_in(community_name: &str) -> String {
    format!("Leave voice in {community_name} first.")
}

/// The live session: its scope, its local controls and the queues and
/// ledgers its loops read.
pub struct LiveVoice {
    pub scope: VoiceScope,
    pub muted: bool,
    pub deafened: bool,
    /// The transport's media roster (arrivals, feedback, allocator).
    pub media: Arc<MediaRoster>,
    /// Media-plane proof of life per sender.
    pub liveness: Arc<MediaLiveness>,
    /// The receive loop's packet channel.
    pub packet_tx: Option<mpsc::Sender<VoicePacket>>,
    /// The send loop's receiver-report channel.
    pub report_tx: Option<mpsc::Sender<VoiceReceiverReport>>,
}

impl LiveVoice {
    /// A session on `scope` with fresh ledgers and no loop channels yet;
    /// its media roster reads `clock` (the Host's clock port).
    #[must_use]
    pub fn new(scope: VoiceScope, clock: Arc<dyn rekindle_utils::Clock>) -> Self {
        Self {
            scope,
            muted: false,
            deafened: false,
            media: Arc::new(MediaRoster::new(clock)),
            liveness: Arc::default(),
            packet_tx: None,
            report_tx: None,
        }
    }
}

/// The Host's voice engine state: one per Host, at most one live session.
pub struct HostVoice {
    live: RwLock<Option<LiveVoice>>,
    drops: AtomicU64,
    clock: Arc<dyn rekindle_utils::Clock>,
}

impl HostVoice {
    #[must_use]
    pub fn new(clock: Arc<dyn rekindle_utils::Clock>) -> Self {
        Self {
            live: RwLock::new(None),
            drops: AtomicU64::new(0),
            clock,
        }
    }

    /// Why a new voice session cannot start now, in the Host's words, or
    /// `None` when idle. `community_name` names a community by id.
    pub fn start_refusal(&self, community_name: impl Fn(&str) -> String) -> Option<String> {
        let live = self.live.read();
        Some(match &live.as_ref()?.scope {
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
        let mut live = self.live.write();
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

    /// End the live session (leave and teardown); its scope, if one ran.
    pub fn end(&self) -> Option<VoiceScope> {
        self.live.write().take().map(|s| s.scope)
    }

    /// The live session's scope.
    #[must_use]
    pub fn session(&self) -> Option<VoiceScope> {
        self.live.read().as_ref().map(|s| s.scope.clone())
    }

    /// Set the live session's mute; `false` when none runs.
    pub fn set_muted(&self, muted: bool) -> bool {
        self.live.write().as_mut().is_some_and(|s| {
            s.muted = muted;
            true
        })
    }

    /// Set the live session's deafen; `false` when none runs.
    pub fn set_deafened(&self, deafened: bool) -> bool {
        self.live.write().as_mut().is_some_and(|s| {
            s.deafened = deafened;
            true
        })
    }

    /// The live session's (muted, deafened), or `None`.
    #[must_use]
    pub fn controls(&self) -> Option<(bool, bool)> {
        self.live.read().as_ref().map(|s| (s.muted, s.deafened))
    }

    /// Inbound voice packets dropped since the Host started.
    #[must_use]
    pub fn voice_drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
}

impl MediaIngress for HostVoice {
    fn media(&self) -> Option<Arc<MediaRoster>> {
        self.live.read().as_ref().map(|s| Arc::clone(&s.media))
    }

    fn note_media_live(&self, sender: &str) {
        if let Some(s) = self.live.read().as_ref() {
            s.liveness.note(sender, self.clock.now_ms());
        }
    }

    fn voice_packet_tx(&self) -> Option<mpsc::Sender<VoicePacket>> {
        self.live.read().as_ref().and_then(|s| s.packet_tx.clone())
    }

    fn receiver_report_tx(&self) -> Option<mpsc::Sender<VoiceReceiverReport>> {
        self.live.read().as_ref().and_then(|s| s.report_tx.clone())
    }

    fn note_voice_drop(&self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
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

#[cfg(test)]
mod tests;
