//! Receive media through the Host session and its shared ingress policy.

use std::sync::Arc;
use std::time::Instant;

use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use rekindle_codec::community::envelope::SignedEnvelope;
use rekindle_voice::ingress::MediaIngress;
use rekindle_voice::receiver_report::VoiceReceiverReport;
use rekindle_voice::transport::{roster::MediaRoster, VoicePacket};
use tokio::sync::mpsc;

use super::HostVoice;
use crate::daemon::dispatch::DaemonContext;

struct Ingress<'a> {
    voice: &'a HostVoice,
    queue: &'a dyn Fn(SignedEnvelope),
}

impl crate::host::Host {
    /// Feed a media datagram to the engine. The video shell supplies its ingress queue.
    #[must_use]
    pub fn ingest_voice_media(
        &self,
        raw: &[u8],
        arrived: Instant,
        queue: impl Fn(SignedEnvelope),
    ) -> bool {
        ingest_media(&self.ctx, raw, arrived, &queue)
    }
}

pub(crate) fn ingest_media(
    ctx: &DaemonContext,
    raw: &[u8],
    arrived: Instant,
    queue: &dyn Fn(SignedEnvelope),
) -> bool {
    if ctx.unlock_scope.read().is_none() {
        return false;
    }
    rekindle_voice::ingress::handle_media_tag(
        &Ingress {
            voice: &ctx.voice,
            queue,
        },
        raw,
        arrived,
    )
}

impl MediaIngress for Ingress<'_> {
    fn media(&self) -> Option<Arc<MediaRoster>> {
        self.voice.media()
    }

    fn note_media_live(&self, sender: &str) {
        if let Some(s) = self.voice.live.lock().as_ref() {
            s.liveness.note(sender, self.voice.clock.now_ms());
        }
    }

    fn voice_packet_tx(&self) -> Option<mpsc::Sender<VoicePacket>> {
        self.voice.packet_tx.read().clone()
    }

    fn receiver_report_tx(&self) -> Option<mpsc::Sender<VoiceReceiverReport>> {
        self.voice.report_tx.read().clone()
    }

    fn note_voice_drop(&self) {
        self.voice.drops.record_ingress();
    }

    fn queue_media_envelope(&self, signed: SignedEnvelope) {
        (self.queue)(signed);
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
        MediaIngress::note_media_live(self, &reporter);
        tracing::debug!(
            peer = %reporter,
            reported = feedback.arrivals.len(),
            estimate_kbps = split.estimate_bps / 1_000,
            audio_bps = split.audio_bps,
            video_bps = split.video_bps,
            feedback_age_ms = self.voice.clock.monotonic().saturating_duration_since(arrived).as_millis(),
            "transport feedback"
        );
    }
}
