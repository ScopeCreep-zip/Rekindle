//! RFC 3550 §6.4 receiver reports for voice — what the receiving end
//! tells the sender about the link.
//!
//! Voice had no return path at all. The send loop could count its own
//! failed `send()` calls and nothing else, so a sender could not tell a
//! clean link from one dropping a fifth of its packets: `send()`
//! succeeds either way. Video has `FrameAck`/`BandwidthEstimate`; this
//! is the voice equivalent, and it carries the two things a sender
//! cannot derive locally.
//!
//! - The receiver's view of the stream —
//!   [`ReceptionMetrics`](rekindle_media_stats::ReceptionMetrics): loss
//!   and discard kept apart, jitter, and the burst/gap split.
//! - The RFC 3550 LSR/DLSR echo, from which the sender computes RTT.
//!
//! ## RTT without a sender report
//!
//! RFC 3550 gets RTT by having the receiver echo the timestamp of the
//! last sender report it saw (`lsr`) together with the delay since it
//! saw it (`dlsr`); the sender then has `rtt = now − lsr − dlsr`. We
//! have no separate sender-report packet — but every [`VoicePacket`]
//! already carries the sender's own `timestamp`, so the echo runs
//! against data packets directly. Same arithmetic, one fewer message
//! type on the wire.
//!
//! The consequence worth stating: `lsr_ms` is in the **sender's** clock
//! (`rekindle_utils::timestamp_ms`, stamped in the send loop) and is
//! only ever subtracted from the sender's own clock, while `dlsr_ms` is
//! a *duration* measured entirely within the receiver. So RTT is
//! computed from one machine's clock plus one machine's stopwatch, and
//! the two are never assumed to agree — which matters, because two
//! peers' wall clocks routinely differ by more than the round trip
//! being measured.
//!
//! ## Why it is signed
//!
//! A report steers the sender's bitrate. An unauthenticated one is
//! therefore a bitrate-floor denial of service: forge
//! `loss_rate_q8: 255` and the sender backs off to
//! `VOICE_MIN_KBPS`-equivalent for as long as the forgery keeps
//! arriving, degrading the call for everyone without ever touching the
//! audio path. Reports carry the same Ed25519 discipline as
//! [`VoicePacket`], under their own domain tag so neither can be
//! replayed as the other.

use crate::error::VoiceError;
use crate::transport::VoicePacket;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rekindle_media_stats::ReceptionMetrics;
use serde::{Deserialize, Serialize};

/// Wire tag for a receiver report, sibling to [`VoicePacket`]'s `b'V'`.
/// Reports ride the same media route as the audio they describe, so
/// they measure the path actually in use.
pub const RECEIVER_REPORT_TAG: u8 = b'R';

/// Domain tag for the report signature. Distinct from
/// `rekindle-voice-packet-v1` so a captured packet signature cannot be
/// presented as a report signature.
const SIGNING_DOMAIN: &[u8] = b"rekindle-voice-receiver-report-v1";

/// One receiver's view of one inbound stream, sent back to that
/// stream's sender.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceReceiverReport {
    /// The reporting receiver's Ed25519 public key (32 bytes). The
    /// signature is verified against this, so a report cannot be
    /// attributed to a peer that did not send it.
    pub reporter_key: Vec<u8>,
    /// Highest sequence number the reporter has seen from us. Lets the
    /// sender distinguish "you are losing packets" from "I have not
    /// heard from you since sequence N".
    pub highest_seq: u32,
    /// The reporter's reception model for our stream.
    pub metrics: ReceptionMetrics,
    /// RFC 3550 LSR: the `timestamp` of the most recent packet the
    /// reporter accepted from us, echoed back unchanged. **In our
    /// clock** — see the module note on clocks.
    pub lsr_ms: u64,
    /// RFC 3550 DLSR: milliseconds between the reporter seeing that
    /// packet and building this report. A duration, measured wholly
    /// within the reporter.
    pub dlsr_ms: u32,
    /// The reporter's current playout depth — RFC 3611's JB nominal.
    /// A sender seeing high discard against a small `jb_nominal_ms`
    /// knows the receiver's buffer is the constraint, not the network.
    pub jb_nominal_ms: u32,
    /// 64-byte Ed25519 signature over [`Self::signing_bytes`].
    pub signature: Vec<u8>,
}

impl VoiceReceiverReport {
    /// Build an unsigned report. Call [`Self::sign`] before sending —
    /// [`Self::from_wire`] rejects anything unsigned.
    #[must_use]
    pub fn new(
        reporter_key: Vec<u8>,
        highest_seq: u32,
        metrics: ReceptionMetrics,
        lsr_ms: u64,
        dlsr_ms: u32,
        jb_nominal_ms: u32,
    ) -> Self {
        Self {
            reporter_key,
            highest_seq,
            metrics,
            lsr_ms,
            dlsr_ms,
            jb_nominal_ms,
            signature: Vec::new(),
        }
    }

    /// Canonical bytes the reporter signs. Every field that a sender
    /// acts on is covered: leaving any metric out would let an attacker
    /// rewrite it in flight while the signature still verified.
    #[must_use]
    pub fn signing_bytes(&self) -> Vec<u8> {
        let m = &self.metrics;
        let mut out = Vec::with_capacity(SIGNING_DOMAIN.len() + self.reporter_key.len() + 48);
        out.extend_from_slice(SIGNING_DOMAIN);
        out.extend_from_slice(&self.reporter_key);
        out.extend_from_slice(&self.highest_seq.to_le_bytes());
        out.extend_from_slice(&self.lsr_ms.to_le_bytes());
        out.extend_from_slice(&self.dlsr_ms.to_le_bytes());
        out.extend_from_slice(&self.jb_nominal_ms.to_le_bytes());
        out.push(m.loss_rate_q8);
        out.push(m.discard_rate_q8);
        out.push(m.burst_density_q8);
        out.push(m.gap_density_q8);
        out.extend_from_slice(&m.burst_duration_ms.to_le_bytes());
        out.extend_from_slice(&m.gap_duration_ms.to_le_bytes());
        out.extend_from_slice(&m.jitter_ms.to_le_bytes());
        out.extend_from_slice(&m.packets_expected.to_le_bytes());
        out.extend_from_slice(&m.packets_received.to_le_bytes());
        out
    }

    /// Sign in place with the reporter's identity key.
    pub fn sign(&mut self, signing_key: &SigningKey) {
        self.signature = signing_key.sign(&self.signing_bytes()).to_bytes().to_vec();
    }

    /// Serialize to wire bytes, `b'R'`-tagged for the dispatch loop.
    pub fn to_wire(&self) -> Result<Vec<u8>, VoiceError> {
        let payload = bincode::serialize(self)
            .map_err(|e| VoiceError::Transport(format!("receiver report encode: {e}")))?;
        let mut data = Vec::with_capacity(1 + payload.len());
        data.push(RECEIVER_REPORT_TAG);
        data.extend_from_slice(&payload);
        Ok(data)
    }

    /// Parse and verify an incoming report.
    ///
    /// Expects data **without** the `b'R'` tag — the dispatch loop
    /// strips it, mirroring [`crate::transport::VoiceTransport::receive`].
    pub fn from_wire(data: &[u8]) -> Result<Self, VoiceError> {
        let report: Self = bincode::deserialize(data)
            .map_err(|e| VoiceError::Transport(format!("receiver report decode: {e}")))?;
        let sig_arr: [u8; 64] = report
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| VoiceError::Transport("receiver report signature length".into()))?;
        let key_arr: [u8; 32] = report
            .reporter_key
            .as_slice()
            .try_into()
            .map_err(|_| VoiceError::Transport("receiver report reporter_key length".into()))?;
        let vk = VerifyingKey::from_bytes(&key_arr).map_err(|e| {
            VoiceError::Transport(format!("receiver report reporter_key invalid: {e}"))
        })?;
        vk.verify_strict(&report.signing_bytes(), &Signature::from_bytes(&sig_arr))
            .map_err(|e| VoiceError::Transport(format!("receiver report signature: {e}")))?;
        Ok(report)
    }

    /// Build and sign a report from a participant's tracked state.
    ///
    /// Returns `None` when the echo has seen no packet, because a
    /// report with nothing to echo tells the sender nothing it does not
    /// already know and would hand it an `lsr_ms: 0` to subtract from.
    #[must_use]
    pub fn build_signed(
        signing_key: &SigningKey,
        reporter_key: Vec<u8>,
        echo: &SenderEcho,
        metrics: ReceptionMetrics,
        jb_nominal_ms: u32,
        local_ms: u64,
    ) -> Option<Self> {
        let (lsr_ms, dlsr_ms) = echo.echo_at(local_ms)?;
        let mut report = Self::new(
            reporter_key,
            echo.highest_seq(),
            metrics,
            lsr_ms,
            dlsr_ms,
            jb_nominal_ms,
        );
        report.sign(signing_key);
        Some(report)
    }

    /// Round-trip time in milliseconds, given the sender's clock now.
    ///
    /// `rtt = now − lsr − dlsr`. Returns `None` when that underflows,
    /// which means the report cannot be believed: either the reporter
    /// echoed an `lsr` we never sent, or it claimed a `dlsr` longer than
    /// the packet has existed. A bogus RTT is worse than none — it would
    /// steer the sender — so this refuses rather than clamping to zero.
    #[must_use]
    pub fn round_trip_ms(&self, now_ms: u64) -> Option<u32> {
        let elapsed = now_ms.checked_sub(self.lsr_ms)?;
        let rtt = elapsed.checked_sub(u64::from(self.dlsr_ms))?;
        u32::try_from(rtt).ok()
    }
}

/// Tracks what a receiver needs in order to fill in a report's LSR and
/// DLSR: the newest sender timestamp seen, and when locally it was
/// seen.
///
/// Kept separate from [`crate::jitter::JitterBuffer`] because it is the
/// only piece of the report that is *not* derivable from the reception
/// model — the buffer measures the stream, this remembers one instant
/// so the sender can measure the path back.
#[derive(Debug, Clone, Default)]
pub struct SenderEcho {
    /// Highest sender timestamp accepted so far, in the sender's clock.
    lsr_ms: u64,
    /// Highest sequence accepted so far.
    highest_seq: u32,
    /// Local time that packet arrived, as milliseconds since some fixed
    /// local origin. Only differences are used, so the origin is free.
    seen_at_ms: u64,
    /// False until the first packet — an all-zero echo would report a
    /// spurious `lsr_ms: 0`.
    seen: bool,
}

impl SenderEcho {
    /// Note an accepted packet. `local_ms` is any monotonic local
    /// millisecond clock; the report only uses differences within it.
    ///
    /// Out-of-order packets do not move the echo backwards: RTT should
    /// be measured against the newest thing we heard, and a reordered
    /// packet would otherwise inflate DLSR by the reorder distance.
    pub fn observe(&mut self, packet: &VoicePacket, local_ms: u64) {
        if self.seen && packet.timestamp <= self.lsr_ms {
            return;
        }
        self.lsr_ms = packet.timestamp;
        self.highest_seq = packet.sequence;
        self.seen_at_ms = local_ms;
        self.seen = true;
    }

    /// Highest sequence seen, for the report.
    #[must_use]
    pub fn highest_seq(&self) -> u32 {
        self.highest_seq
    }

    /// The `(lsr_ms, dlsr_ms)` pair to put in a report built at
    /// `local_ms`, or `None` if no packet has been seen yet — in which
    /// case there is nothing to echo and no report is worth sending.
    #[must_use]
    pub fn echo_at(&self, local_ms: u64) -> Option<(u64, u32)> {
        if !self.seen {
            return None;
        }
        let dlsr = local_ms.saturating_sub(self.seen_at_ms);
        Some((self.lsr_ms, u32::try_from(dlsr).unwrap_or(u32::MAX)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn metrics() -> ReceptionMetrics {
        ReceptionMetrics {
            loss_rate_q8: 13,
            discard_rate_q8: 4,
            burst_density_q8: 90,
            gap_density_q8: 1,
            burst_duration_ms: 120,
            gap_duration_ms: 4200,
            jitter_ms: 27,
            packets_expected: 500,
            packets_received: 487,
        }
    }

    fn signed_report() -> VoiceReceiverReport {
        let sk = key();
        let mut r = VoiceReceiverReport::new(
            sk.verifying_key().to_bytes().to_vec(),
            1234,
            metrics(),
            9_000,
            15,
            60,
        );
        r.sign(&sk);
        r
    }

    fn packet(seq: u32, ts: u64) -> VoicePacket {
        VoicePacket {
            sender_key: vec![0; 32],
            sequence: seq,
            timestamp: ts,
            audio_data: vec![0; 8],
            mek_generation: 0,
            signature: Vec::new(),
        }
    }

    #[test]
    fn round_trips_through_the_wire_with_the_tag_stripped() {
        let wire = signed_report().to_wire().unwrap();
        assert_eq!(wire[0], RECEIVER_REPORT_TAG, "tagged for the dispatch loop");
        let back = VoiceReceiverReport::from_wire(&wire[1..]).unwrap();
        assert_eq!(back.metrics, metrics());
        assert_eq!(back.highest_seq, 1234);
        assert_eq!(back.jb_nominal_ms, 60);
    }

    #[test]
    fn unsigned_report_is_rejected() {
        let r = VoiceReceiverReport::new(
            key().verifying_key().to_bytes().to_vec(),
            1,
            metrics(),
            0,
            0,
            40,
        );
        let wire = r.to_wire().unwrap();
        assert!(
            VoiceReceiverReport::from_wire(&wire[1..]).is_err(),
            "an unsigned report must not be actionable"
        );
    }

    #[test]
    fn every_actionable_field_is_covered_by_the_signature() {
        // The DoS this prevents: rewrite a metric in flight and have
        // the sender still believe it. Each mutation must break verify.
        let base = signed_report();
        let mut cases: Vec<(&str, VoiceReceiverReport)> = Vec::new();

        let mut loss = base.clone();
        loss.metrics.loss_rate_q8 = 255;
        cases.push(("loss_rate_q8", loss));

        let mut discard = base.clone();
        discard.metrics.discard_rate_q8 = 255;
        cases.push(("discard_rate_q8", discard));

        let mut jitter = base.clone();
        jitter.metrics.jitter_ms = 9_999;
        cases.push(("jitter_ms", jitter));

        let mut burst = base.clone();
        burst.metrics.burst_density_q8 = 255;
        cases.push(("burst_density_q8", burst));

        let mut gap = base.clone();
        gap.metrics.gap_density_q8 = 255;
        cases.push(("gap_density_q8", gap));

        let mut burst_ms = base.clone();
        burst_ms.metrics.burst_duration_ms = 60_000;
        cases.push(("burst_duration_ms", burst_ms));

        let mut gap_ms = base.clone();
        gap_ms.metrics.gap_duration_ms = 1;
        cases.push(("gap_duration_ms", gap_ms));

        let mut expected = base.clone();
        expected.metrics.packets_expected = 1;
        cases.push(("packets_expected", expected));

        let mut received = base.clone();
        received.metrics.packets_received = 1;
        cases.push(("packets_received", received));

        let mut seq = base.clone();
        seq.highest_seq = 0;
        cases.push(("highest_seq", seq));

        let mut lsr = base.clone();
        lsr.lsr_ms = 1;
        cases.push(("lsr_ms", lsr));

        let mut dlsr = base.clone();
        dlsr.dlsr_ms = 5_000;
        cases.push(("dlsr_ms", dlsr));

        let mut jb = base.clone();
        jb.jb_nominal_ms = 120;
        cases.push(("jb_nominal_ms", jb));

        for (field, tampered) in cases {
            let wire = tampered.to_wire().unwrap();
            assert!(
                VoiceReceiverReport::from_wire(&wire[1..]).is_err(),
                "tampering with {field} must invalidate the signature"
            );
        }
    }

    #[test]
    fn report_signed_by_one_peer_cannot_be_attributed_to_another() {
        let mut r = signed_report();
        // Swap in a different reporter key, keeping the signature.
        r.reporter_key = SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes()
            .to_vec();
        let wire = r.to_wire().unwrap();
        assert!(VoiceReceiverReport::from_wire(&wire[1..]).is_err());
    }

    #[test]
    fn packet_signature_cannot_be_replayed_as_a_report_signature() {
        // Both sign with the same identity key; only the domain tag
        // separates them.
        let sk = key();
        let p = packet(1, 20);
        let packet_sig = sk.sign(&p.signing_bytes());
        let mut r = VoiceReceiverReport::new(
            sk.verifying_key().to_bytes().to_vec(),
            1,
            metrics(),
            20,
            0,
            40,
        );
        r.signature = packet_sig.to_bytes().to_vec();
        let wire = r.to_wire().unwrap();
        assert!(VoiceReceiverReport::from_wire(&wire[1..]).is_err());
    }

    #[test]
    fn rtt_subtracts_the_receivers_own_delay() {
        let r = signed_report(); // lsr 9_000, dlsr 15
                                 // Sender's clock reads 9_100: 100 ms round trip, of which 15
                                 // was the receiver sitting on the report.
        assert_eq!(r.round_trip_ms(9_100), Some(85));
    }

    #[test]
    fn impossible_rtt_is_refused_not_clamped() {
        let r = signed_report(); // lsr 9_000, dlsr 15
                                 // An lsr from the future.
        assert_eq!(r.round_trip_ms(8_000), None);
        // A dlsr longer than the packet has existed.
        assert_eq!(r.round_trip_ms(9_010), None);
    }

    #[test]
    fn echo_tracks_the_newest_packet_and_ignores_reordering() {
        let mut echo = SenderEcho::default();
        assert_eq!(echo.echo_at(500), None, "nothing to echo yet");

        echo.observe(&packet(10, 200), 1_000);
        assert_eq!(echo.echo_at(1_050), Some((200, 50)));
        assert_eq!(echo.highest_seq(), 10);

        // A reordered older packet must not drag the echo back — doing
        // so would inflate DLSR by the reorder distance.
        echo.observe(&packet(9, 180), 1_060);
        assert_eq!(echo.echo_at(1_050), Some((200, 50)));
        assert_eq!(echo.highest_seq(), 10);

        echo.observe(&packet(11, 220), 1_070);
        assert_eq!(echo.echo_at(1_100), Some((220, 30)));
        assert_eq!(echo.highest_seq(), 11);
    }
}
