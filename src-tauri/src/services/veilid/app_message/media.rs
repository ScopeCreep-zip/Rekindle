//! The two media fast paths off the app_message dispatch: inbound
//! voice packets, and the RFC 3550 receiver reports that flow back the
//! other way.
//!
//! Both are handled the same way and for the same reason — verify,
//! `try_send` onto the owning loop's channel, done. Dispatch is serial
//! across every app_message the node receives, so anything slower here
//! stalls audio for the whole session; that is why the gossip/video
//! path below this is queued to a worker rather than run inline.

use std::sync::Arc;

use crate::state::AppState;

/// Handle a media datagram: a `b'V'` voice packet, `b'M'` video-plane
/// envelope or `b'P'` padding (each after its route `transport_seq`,
/// plans E4.3.1, E4.3.3), a
/// `b'T'` transport feedback report (plan E4.3.2), or a `b'R'` receiver
/// report. Returns `true` if the message was one of them and is now
/// dealt with.
pub(super) fn handle_media_tag(state: &Arc<AppState>, message: &[u8]) -> bool {
    // Arrival is stamped here, on the dispatch thread, before any queue
    // adds its own delay to what the estimator measures.
    let arrived = std::time::Instant::now();
    if let Some((tag, transport_seq, payload)) =
        rekindle_voice::media_frame::split_sequenced(message)
    {
        match tag {
            rekindle_voice::media_frame::VOICE_TAG => {
                handle_voice_packet(state, transport_seq, payload, arrived);
            }
            rekindle_voice::media_frame::PADDING_TAG => {
                handle_padding(state, transport_seq, payload, arrived);
            }
            _ => handle_media_envelope(state, transport_seq, payload, arrived),
        }
        return true;
    }
    if message.first() == Some(&rekindle_voice::media_frame::FEEDBACK_TAG) {
        handle_transport_feedback(state, &message[1..], arrived);
        return true;
    }

    // RFC 3550 receiver report — the return direction of the voice fast
    // path above, and handled the same way: verify + `try_send`, never
    // any work on the dispatch thread. Routed to the SEND loop, because
    // a report is about our outbound stream.
    if !message.is_empty() && message[0] == rekindle_voice::receiver_report::RECEIVER_REPORT_TAG {
        match rekindle_voice::receiver_report::VoiceReceiverReport::from_wire(&message[1..]) {
            Ok(report) => {
                let tx = state.voice_report_tx.read().clone();
                match tx {
                    Some(tx) if tx.try_send(report).is_err() => {
                        tracing::debug!("receiver report channel full, dropping report");
                    }
                    Some(_) => {}
                    None => {
                        tracing::debug!("receiver report arrived with no voice send loop running");
                    }
                }
            }
            Err(e) => {
                // Unsigned or forged. Dropping is the whole point: a
                // report steers our bitrate, so an unauthenticated one
                // is a bitrate-floor DoS lever.
                tracing::info!(error = %e, "receiver report rejected (deserialize/sig)");
            }
        }
        return true;
    }

    false
}

/// A voice packet: verified, recorded for transport feedback, then handed
/// to the receive loop.
fn handle_voice_packet(
    state: &Arc<AppState>,
    transport_seq: u32,
    voice_data: &[u8],
    arrived: std::time::Instant,
) {
    match rekindle_voice::transport::VoiceTransport::receive(voice_data) {
        Ok(packet) => {
            let sender = hex::encode(&packet.sender_key);
            crate::state_helpers::note_media_live(state, &sender);
            if let Some(media) = crate::state_helpers::voice_media(state) {
                media
                    .arrivals()
                    .record_voice(&sender, packet.sequence, transport_seq, arrived);
            }
            let tx = state.voice_packet_tx.read().clone();
            if let Some(tx) = tx {
                if tx.try_send(packet).is_err() {
                    // W14.4 — was trace! (invisible). Channel
                    // full = real backpressure; surface it.
                    tracing::warn!("voice packet channel full, dropping packet");
                    state
                        .voice_pkt_drops
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            } else {
                // No voice session takes packets (none started yet, or
                // the peer has not yet seen us leave).
                tracing::debug!("voice packet with no voice session — dropping");
                state
                    .voice_pkt_drops
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        Err(e) => {
            // Deserialize / signature verify failure. Could be a
            // forged packet or a stale-bytes-on-route artifact.
            tracing::info!(error = %e, "voice packet rejected (deserialize/sig)");
            state
                .voice_pkt_drops
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Padding the sender's pacer sent to probe the route: counted for its
/// sender's feedback if its signature binds it to this sequence number,
/// then discarded.
fn handle_padding(
    state: &Arc<AppState>,
    transport_seq: u32,
    payload: &[u8],
    arrived: std::time::Instant,
) {
    let Some(sender) = rekindle_voice::media_frame::open_padding(transport_seq, payload) else {
        tracing::debug!("padding with no valid signature — dropping");
        return;
    };
    let sender = hex::encode(sender);
    crate::state_helpers::note_media_live(state, &sender);
    if let Some(media) = crate::state_helpers::voice_media(state) {
        media.arrivals().record(&sender, transport_seq, arrived);
    }
}

/// A video-plane envelope on the media route: queued to the gossip
/// ingress worker like any signed envelope, carrying its arrival so the
/// worker records it for feedback once the signature verifies.
fn handle_media_envelope(
    state: &Arc<AppState>,
    transport_seq: u32,
    envelope: &[u8],
    arrived: std::time::Instant,
) {
    let Ok(signed) = rekindle_codec::capnp_envelope::decode_signed_envelope(envelope) else {
        tracing::debug!("media envelope did not decode — dropping");
        return;
    };
    let is_video = super::video_payload_channel_from_bytes(&signed.envelope_bytes).is_some();
    state.gossip_ingress.push(
        crate::services::veilid::ingress_queue::IngressItem::Gossip {
            signed,
            is_video,
            media_arrival: Some((transport_seq, arrived)),
        },
    );
}

/// Transport feedback about our outbound stream to one peer: verified,
/// then matched against that route's send history (plan E4.3.2).
fn handle_transport_feedback(state: &Arc<AppState>, data: &[u8], arrived: std::time::Instant) {
    let feedback =
        match rekindle_codec::capnp_codec::transport_feedback::TransportFeedback::decode(data) {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!(error = %e, "transport feedback did not decode — dropping");
                return;
            }
        };
    if let Err(e) = rekindle_codec::capnp_codec::SignedWire::verify(&feedback) {
        // Feedback steers our send rate: an unauthenticated report is a
        // rate lever for anyone holding our route.
        tracing::info!(error = %e, "transport feedback rejected (signature)");
        return;
    }
    crate::services::voice_adapter::media_feedback::on_transport_feedback(
        state, &feedback, arrived,
    );
}
