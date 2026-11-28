//! Exercises verified datagram handoff and refusal.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use parking_lot::Mutex;
use rekindle_codec::capnp_codec::{transport_feedback::TransportFeedback, SignedWire};
use rekindle_codec::capnp_envelope::encode_signed_envelope;
use rekindle_codec::community::envelope::{sign_envelope, SignedEnvelope};
use rekindle_media_stats::ReceptionMetrics;
use tokio::sync::mpsc;

use super::{handle_media_tag, MediaIngress};
use crate::media_frame::{self, FEEDBACK_TAG, PADDING_TAG, VOICE_TAG};
use crate::receiver_report::{VoiceReceiverReport, RECEIVER_REPORT_TAG};
use crate::transport::roster::MediaRoster;
use crate::transport::VoicePacket;

/// Records what the policy asked of the host.
struct Host {
    media: Arc<MediaRoster>,
    live: Mutex<Vec<String>>,
    drops: AtomicUsize,
    queued: AtomicUsize,
    feedback: AtomicUsize,
    packets: Mutex<Option<mpsc::Sender<VoicePacket>>>,
    reports: Mutex<Option<mpsc::Sender<VoiceReceiverReport>>>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            media: Arc::new(MediaRoster::new(Arc::new(rekindle_utils::SystemClock))),
            live: Mutex::default(),
            drops: AtomicUsize::default(),
            queued: AtomicUsize::default(),
            feedback: AtomicUsize::default(),
            packets: Mutex::default(),
            reports: Mutex::default(),
        }
    }
}

impl MediaIngress for Host {
    fn media(&self) -> Option<Arc<MediaRoster>> {
        Some(Arc::clone(&self.media))
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
    let key = SigningKey::from_bytes(&[9; 32]);
    let mut packet = signed_voice_packet(&key);
    packet.timestamp += 1;
    let message = media_frame::sequenced(VOICE_TAG, 7, &packet.encode().unwrap());
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
    let key = SigningKey::from_bytes(&[4; 32]);
    let mut message = media_frame::sequenced(PADDING_TAG, 3, &[0; media_frame::PADDING_HEADER_LEN]);
    assert!(media_frame::sign_padding(&mut message, &key));
    message[1] = 4;
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

    let key = SigningKey::from_bytes(&[4; 32]);
    let mut envelope = sign_envelope(
        &key,
        &"ab".repeat(32),
        &hex::encode(key.verifying_key().to_bytes()),
        [1; 16],
        b"video",
    );
    // The worker surfaces rejected video. The fast path credits only verified arrivals.
    envelope.signature[0] ^= 1;
    let bytes = encode_signed_envelope(&envelope).unwrap();
    let message = media_frame::sequenced(media_frame::ENVELOPE_TAG, 2, &bytes);
    assert!(handle_media_tag(&host, &message, Instant::now()));
    assert_eq!(host.queued.load(Ordering::Relaxed), 1);
    assert!(host.live.lock().is_empty());
    assert!(host.media.arrivals().peers_with_arrivals().is_empty());
}

/// Feedback and receiver reports steer our send rate: an unverified one
/// reaches neither the feedback handler nor the send loop.
#[test]
fn unverified_rate_reports_are_dropped() {
    let host = Host::default();
    let (tx, mut rx) = mpsc::channel(4);
    *host.reports.lock() = Some(tx);
    let key = SigningKey::from_bytes(&[7; 32]);
    let reporter_key = key.verifying_key().to_bytes().to_vec();
    let mut feedback = TransportFeedback {
        reporter_key: reporter_key.clone(),
        begin_seq: 3,
        report_time_ms: 120,
        arrivals: vec![1],
        sig: Vec::new(),
    };
    feedback.sign(&key);
    feedback.report_time_ms += 1;
    let mut bytes = vec![FEEDBACK_TAG];
    bytes.extend(feedback.encode().unwrap());
    assert!(handle_media_tag(&host, &bytes, Instant::now()));

    let mut report =
        VoiceReceiverReport::new(reporter_key, 42, ReceptionMetrics::default(), 120, 5, 10);
    report.sign(&key);
    report.highest_seq += 1;
    let bytes = report.to_wire().unwrap();
    assert_eq!(bytes.first(), Some(&RECEIVER_REPORT_TAG));
    assert!(handle_media_tag(&host, &bytes, Instant::now()));
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

fn signed_voice_packet(key: &SigningKey) -> VoicePacket {
    let mut packet = VoicePacket {
        sender_key: key.verifying_key().to_bytes().to_vec(),
        sequence: 9,
        timestamp: 50,
        sframe: vec![7; 32],
        sig: Vec::new(),
    };
    packet.sign(key);
    packet
}

#[test]
fn verified_voice_is_recorded_at_receipt_even_when_its_queue_is_full() {
    let host = Host::default();
    let (tx, mut rx) = mpsc::channel(1);
    *host.packets.lock() = Some(tx);
    let now = Instant::now();
    let arrived = now.checked_sub(Duration::from_millis(125)).unwrap();
    let key = SigningKey::from_bytes(&[5; 32]);
    let mut packet = signed_voice_packet(&key);
    let sender = hex::encode(&packet.sender_key);
    let message = media_frame::sequenced(VOICE_TAG, 100, &packet.encode().unwrap());
    assert!(handle_media_tag(&host, &message, arrived));

    packet.sequence += 1;
    packet.sign(&key);
    let message = media_frame::sequenced(VOICE_TAG, 101, &packet.encode().unwrap());
    assert!(handle_media_tag(&host, &message, arrived));
    assert_eq!(host.drops.load(Ordering::Relaxed), 1);
    assert_eq!(rx.try_recv().unwrap().sequence, 9);
    assert_eq!(
        host.live.lock().as_slice(),
        &[sender.clone(), sender.clone()]
    );
    let report = host.media.arrivals().take_report(&sender, now).unwrap();
    assert_eq!(report.begin_seq, 100);
    assert_eq!(report.arrivals, vec![128, 128]);
}
