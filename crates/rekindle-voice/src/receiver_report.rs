//! RFC 3550 §6.4 receiver reports for voice — what the receiving end
//! tells the sender about the link.
//!
//! Voice had no return path at all. The send loop could count its own
//! failed `send()` calls and nothing else, so a sender could not tell a
//! clean link from one dropping a fifth of its packets: `send()`
//! succeeds either way. Reports carry the two things a sender cannot
//! derive locally; the media route's transport feedback (plan E4.3.3)
//! carries per-datagram arrivals for the bandwidth estimator.
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
use rekindle_types::domains;
use serde::{Deserialize, Serialize};

/// Wire tag for a receiver report, sibling to [`VoicePacket`]'s `b'V'`;
/// it lives with the other ingress tags (plan E4.1 S3).
pub use rekindle_codec::ingress::RECEIVER_REPORT_TAG;

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
        let mut out =
            Vec::with_capacity(domains::VOICE_RECEIVER_REPORT.len() + self.reporter_key.len() + 48);
        out.extend_from_slice(domains::VOICE_RECEIVER_REPORT.as_bytes());
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
        out.extend_from_slice(&m.delay_p50_ms.to_le_bytes());
        out.extend_from_slice(&m.delay_p95_ms.to_le_bytes());
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
        let payload = postcard::to_allocvec(self)
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
        let report: Self = postcard::from_bytes(data)
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
#[path = "receiver_report_tests.rs"]
mod tests;
