//! The ingress policy's refusals (plan E4.1): nothing unverified reaches a
//! loop, a ledger or the gossip worker, and every dropped voice packet is
//! counted.

use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::Mutex;

use super::*;

/// Records what the policy asked of the host.
#[derive(Default)]
struct Host {
    live: Mutex<Vec<String>>,
    drops: AtomicUsize,
    queued: AtomicUsize,
    feedback: AtomicUsize,
    packets: Mutex<Option<mpsc::Sender<VoicePacket>>>,
    reports: Mutex<Option<mpsc::Sender<VoiceReceiverReport>>>,
}

impl MediaIngress for Host {
    fn media(&self) -> Option<Arc<MediaRoster>> {
        None
    }
    fn note_media_live(&self, sender: &str) {
        self.live.lock().push(sender.to_string());
    }
    fn voice_packet_tx(&self) -> Option<mpsc::Sender<VoicePacket>> {
        self.packets.lock().clone()
    }
    fn receiver_report_tx(&self) -> Option<mpsc::Sender<VoiceReceiverReport>> {
        self.reports.lock().clone()
    }
    fn note_voice_drop(&self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
    fn queue_media_envelope(&self, _: SignedEnvelope) {
        self.queued.fetch_add(1, Ordering::Relaxed);
    }
    fn on_transport_feedback(&self, _: &TransportFeedback, _: Instant) {
        self.feedback.fetch_add(1, Ordering::Relaxed);
    }
}

/// A forged voice packet is dealt with, never handed to the receive loop,
/// never proof of life, and counted as a drop.
#[test]
fn a_forged_voice_packet_is_dropped_and_counted() {
    let host = Host::default();
    let (tx, mut rx) = mpsc::channel(4);
    *host.packets.lock() = Some(tx);
    let message = media_frame::sequenced(VOICE_TAG, 7, b"not a signed voice packet");
    assert!(handle_media_tag(&host, &message, Instant::now()));
    assert!(rx.try_recv().is_err());
    assert!(host.live.lock().is_empty());
    assert_eq!(host.drops.load(Ordering::Relaxed), 1);
}

/// Padding without a signature binding it to its sequence number counts
/// for no one.
#[test]
fn unsigned_padding_is_discarded() {
    let host = Host::default();
    let message = media_frame::sequenced(PADDING_TAG, 3, &[0; 64]);
    assert!(handle_media_tag(&host, &message, Instant::now()));
    assert!(host.live.lock().is_empty());
}

/// An envelope that does not decode is not queued to the gossip worker.
#[test]
fn an_undecodable_media_envelope_is_not_queued() {
    let host = Host::default();
    let message = media_frame::sequenced(media_frame::ENVELOPE_TAG, 1, b"garbage");
    assert!(handle_media_tag(&host, &message, Instant::now()));
    assert_eq!(host.queued.load(Ordering::Relaxed), 0);
}

/// Feedback and receiver reports steer our send rate: an unverified one
/// reaches neither the feedback handler nor the send loop.
#[test]
fn unverified_rate_reports_are_dropped() {
    let host = Host::default();
    let (tx, mut rx) = mpsc::channel(4);
    *host.reports.lock() = Some(tx);
    assert!(handle_media_tag(
        &host,
        &[FEEDBACK_TAG, 1, 2, 3],
        Instant::now()
    ));
    assert!(handle_media_tag(
        &host,
        &[RECEIVER_REPORT_TAG, 1, 2, 3],
        Instant::now()
    ));
    assert_eq!(host.feedback.load(Ordering::Relaxed), 0);
    assert!(rx.try_recv().is_err());
}

/// Anything else is not media and goes on to the rest of the dispatch.
#[test]
fn other_messages_are_not_media() {
    let host = Host::default();
    assert!(!handle_media_tag(&host, b"D-frame", Instant::now()));
    assert!(!handle_media_tag(&host, &[], Instant::now()));
}
