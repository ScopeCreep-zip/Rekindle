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

/// Handle a `b'V'` voice packet or a `b'R'` receiver report.
/// Returns `true` if the message was one of them and is now dealt with.
pub(super) fn handle_media_tag(state: &Arc<AppState>, message: &[u8]) -> bool {
    if !message.is_empty() && message[0] == b'V' {
        let voice_data = &message[1..];
        match rekindle_voice::transport::VoiceTransport::receive(voice_data) {
            Ok(packet) => {
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
                    // W14.4 — voice_packet_tx is None. Once W14.1's
                    // permanent-ingress refactor lands this branch
                    // is unreachable. Until then, the lazy-init
                    // race (caller-side post-CallAccept) drops
                    // packets here.
                    tracing::warn!(
                            "voice packet arrived before voice session was set up — dropping (W14.1 will fix)"
                        );
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
