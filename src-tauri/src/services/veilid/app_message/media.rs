//! Bind voice ingress to the desktop's current session and work queues.

use std::sync::Arc;
use std::time::Instant;

use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use rekindle_codec::community::envelope::SignedEnvelope;
use rekindle_transport::subscriptions::lanes::IngressItem;
use rekindle_voice::ingress::{self, MediaIngress};
use rekindle_voice::receiver_report::VoiceReceiverReport;
use rekindle_voice::transport::{roster::MediaRoster, VoicePacket};
use tokio::sync::mpsc;

use crate::{state::AppState, state_helpers};

struct DesktopMediaIngress<'a>(&'a Arc<AppState>);

impl MediaIngress for DesktopMediaIngress<'_> {
    fn media(&self) -> Option<Arc<MediaRoster>> {
        state_helpers::voice_media(self.0)
    }

    fn note_media_live(&self, sender: &str) {
        state_helpers::note_media_live(self.0, sender);
    }

    fn voice_packet_tx(&self) -> Option<mpsc::Sender<VoicePacket>> {
        self.0.voice_packet_tx.read().clone()
    }

    fn receiver_report_tx(&self) -> Option<mpsc::Sender<VoiceReceiverReport>> {
        self.0.voice_report_tx.read().clone()
    }

    fn note_voice_drop(&self) {
        self.0.voice_packet_drops.record_ingress();
    }

    fn queue_media_envelope(&self, signed: SignedEnvelope) {
        let is_video =
            rekindle_video::video_payload_channel_from_bytes(&signed.envelope_bytes).is_some();
        self.0
            .gossip_ingress
            .push(IngressItem::Gossip { signed, is_video });
    }

    fn on_transport_feedback(&self, feedback: &TransportFeedback, arrived: Instant) {
        crate::services::voice_adapter::media_feedback::on_transport_feedback(
            self.0, feedback, arrived,
        );
    }
}

pub(super) fn handle_media_tag(state: &Arc<AppState>, message: &[u8], arrived: Instant) -> bool {
    ingress::handle_media_tag(&DesktopMediaIngress(state), message, arrived)
}
