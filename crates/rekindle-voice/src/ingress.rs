//! The media fast paths off the app_message dispatch (plan E4.1, moved
//! from `src-tauri/src/services/veilid/app_message/media.rs`): inbound
//! voice packets, padding, video-plane envelopes on the media route,
//! transport feedback, and the RFC 3550 receiver reports that flow back
//! the other way.
//!
//! Each is handled the same way and for the same reason: verify, record
//! the arrival, `try_send` onto the owning loop's channel, done. Dispatch
//! is serial across every app_message the node receives, so anything
//! slower here stalls audio for the whole session; that is why the
//! video-plane envelope is queued to a worker rather than handled inline.
//! Nothing here awaits.

use std::sync::Arc;
use std::time::Instant;

use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use rekindle_codec::community::envelope::SignedEnvelope;
use tokio::sync::mpsc;

use crate::media_frame::{self, FEEDBACK_TAG, PADDING_TAG, VOICE_TAG};
use crate::receiver_report::{VoiceReceiverReport, RECEIVER_REPORT_TAG};
use crate::transport::roster::MediaRoster;
use crate::transport::{VoicePacket, VoiceTransport};

/// What the ingress policy reaches in the host: the running session's
/// queues and ledgers. Every method is synchronous.
pub trait MediaIngress {
    /// The running voice session's media roster, if one runs.
    fn media(&self) -> Option<Arc<MediaRoster>>;
    /// A verified datagram from `sender` (hex key or pseudonym): media-plane
    /// proof that it is alive.
    fn note_media_live(&self, sender: &str);
    /// The receive loop's packet channel, if a session takes packets.
    fn voice_packet_tx(&self) -> Option<mpsc::Sender<VoicePacket>>;
    /// The send loop's receiver-report channel, if a send loop runs.
    fn receiver_report_tx(&self) -> Option<mpsc::Sender<VoiceReceiverReport>>;
    /// An inbound voice packet was dropped (forged, no session, or a full
    /// channel).
    fn note_voice_drop(&self);
    /// Queue a decoded envelope to the worker for signature enforcement
    /// and delivery or scoped rejection.
    fn queue_media_envelope(&self, signed: SignedEnvelope);
    /// A verified transport feedback report about our outbound stream.
    fn on_transport_feedback(&self, feedback: &TransportFeedback, arrived: Instant);
}

/// Handle a media datagram that `arrived` (stamped on the dispatch thread,
/// before any queue adds its own delay to what the estimator measures): a
/// `b'V'` voice packet, `b'M'` video-plane envelope or `b'P'` padding (each
/// after its route `transport_seq`, plans E4.3.1, E4.3.3), a `b'T'`
/// transport feedback report (plan E4.3.2), or a `b'R'` receiver report.
/// Returns `true` if the message was one of them and is now dealt with.
pub fn handle_media_tag(ingress: &impl MediaIngress, message: &[u8], arrived: Instant) -> bool {
    if let Some((tag, transport_seq, payload)) = media_frame::split_sequenced(message) {
        match tag {
            VOICE_TAG => handle_voice_packet(ingress, transport_seq, payload, arrived),
            PADDING_TAG => handle_padding(ingress, transport_seq, payload, arrived),
            _ => handle_media_envelope(ingress, transport_seq, payload, arrived),
        }
        return true;
    }
    match message.split_first() {
        Some((&FEEDBACK_TAG, data)) => {
            handle_transport_feedback(ingress, data, arrived);
            true
        }
        // The return direction of the voice fast path, handled the same
        // way. Routed to the SEND loop: a report is about our outbound
        // stream.
        Some((&RECEIVER_REPORT_TAG, data)) => {
            handle_receiver_report(ingress, data);
            true
        }
        _ => false,
    }
}

fn handle_receiver_report(ingress: &impl MediaIngress, data: &[u8]) {
    match VoiceReceiverReport::from_wire(data) {
        Ok(report) => match ingress.receiver_report_tx() {
            Some(tx) if tx.try_send(report).is_err() => {
                tracing::debug!("receiver report channel full, dropping report");
            }
            Some(_) => {}
            None => tracing::debug!("receiver report arrived with no voice send loop running"),
        },
        // Unsigned or forged. Dropping is the whole point: a report steers
        // our bitrate, so an unauthenticated one is a bitrate-floor DoS
        // lever.
        Err(e) => tracing::info!(error = %e, "receiver report rejected (deserialize/sig)"),
    }
}

/// A voice packet: verified, recorded for transport feedback, then handed
/// to the receive loop.
fn handle_voice_packet(
    ingress: &impl MediaIngress,
    transport_seq: u32,
    voice_data: &[u8],
    arrived: Instant,
) {
    let packet = match VoiceTransport::receive(voice_data) {
        Ok(packet) => packet,
        Err(e) => {
            // Deserialize / signature failure: a forged packet or stale
            // bytes on the route.
            tracing::info!(error = %e, "voice packet rejected (deserialize/sig)");
            ingress.note_voice_drop();
            return;
        }
    };
    let sender = hex::encode(&packet.sender_key);
    ingress.note_media_live(&sender);
    if let Some(media) = ingress.media() {
        media
            .arrivals()
            .record_voice(&sender, packet.sequence, transport_seq, arrived);
    }
    let Some(tx) = ingress.voice_packet_tx() else {
        // No voice session takes packets (none started yet, or the peer
        // has not yet seen us leave).
        tracing::debug!("voice packet with no voice session — dropping");
        ingress.note_voice_drop();
        return;
    };
    if tx.try_send(packet).is_err() {
        // A full channel is real backpressure: surface it.
        tracing::warn!("voice packet channel full, dropping packet");
        ingress.note_voice_drop();
    }
}

/// Padding the sender's pacer sent to probe the route: counted for its
/// sender's feedback if its signature binds it to this sequence number,
/// then discarded.
fn handle_padding(
    ingress: &impl MediaIngress,
    transport_seq: u32,
    payload: &[u8],
    arrived: Instant,
) {
    let Some(sender) = media_frame::open_padding(transport_seq, payload) else {
        tracing::debug!("padding with no valid signature — dropping");
        return;
    };
    let sender = hex::encode(sender);
    ingress.note_media_live(&sender);
    if let Some(media) = ingress.media() {
        media.arrivals().record(&sender, transport_seq, arrived);
    }
}

/// A video-plane envelope on the media route. Its arrival is recorded for
/// transport feedback here, on the dispatch thread, once its signature
/// verifies (only a sender's own envelopes count toward its feedback, r6
/// R-BW6); then it is queued to the gossip ingress worker like any signed
/// envelope. Recorded behind the queue, it could land after a report had
/// already counted it lost (plan E4.3.3).
fn handle_media_envelope(
    ingress: &impl MediaIngress,
    transport_seq: u32,
    envelope: &[u8],
    arrived: Instant,
) {
    let Ok(signed) = rekindle_codec::capnp_envelope::decode_signed_envelope(envelope) else {
        tracing::debug!("media envelope did not decode — dropping");
        return;
    };
    if rekindle_codec::community::envelope::verify_envelope(&signed).is_ok() {
        ingress.note_media_live(&signed.sender_pseudonym);
        if let Some(media) = ingress.media() {
            media.arrivals().record_signed(
                &signed.sender_pseudonym,
                &signed.signature,
                transport_seq,
                arrived,
            );
        }
    }
    ingress.queue_media_envelope(signed);
}

/// Transport feedback about our outbound stream to one peer: verified,
/// then matched against that route's send history (plan E4.3.2).
fn handle_transport_feedback(ingress: &impl MediaIngress, data: &[u8], arrived: Instant) {
    let rejected = || {
        if let Some(media) = ingress.media() {
            media.feedback_stats().note_rejected();
        }
    };
    let feedback = match TransportFeedback::decode(data) {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!(error = %e, "transport feedback did not decode — dropping");
            rejected();
            return;
        }
    };
    if let Err(e) = rekindle_codec::capnp_codec::SignedWire::verify(&feedback) {
        // Feedback steers our send rate: an unauthenticated report is a
        // rate lever for anyone holding our route.
        tracing::info!(error = %e, "transport feedback rejected (signature)");
        rejected();
        return;
    }
    ingress.on_transport_feedback(&feedback, arrived);
}

#[cfg(test)]
mod tests;
