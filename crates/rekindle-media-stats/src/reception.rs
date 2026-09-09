//! Receiver-side stream metrics — the RFC 3611 VoIP Metrics model.
//!
//! Fed one call to [`ReceptionTracker::on_packet`] per arriving packet,
//! it produces the numbers a telecom actually watches: loss separated
//! from discard, jitter, and — the one that decides whether a call
//! *sounds* broken — the burst/gap split.
//!
//! ## Why loss rate alone is not enough
//!
//! 5 % loss spread evenly is a call with a faint texture to it. 5 % loss
//! arriving as one 400 ms hole is a call that dropped a word. Both
//! report "5 %". [RFC 3611 §4.7] separates them with a **burst/gap**
//! model: a *burst* is a period bracketed by lost packets containing no
//! run of `Gmin` or more consecutive received packets; everything else
//! is a *gap*. Density is then reported for each period separately.
//!
//! Gmin defaults to 16, which RFC 3611 notes "corresponds to a burst
//! period having a minimum density of 6.25 % of lost or discarded
//! packets".
//!
//! ## Loss versus discard
//!
//! [RFC 3611] keeps these apart and so do we, because they have
//! different causes and different fixes:
//!
//! - **loss** — the packet never arrived. A network problem: route,
//!   congestion, a relay dropping.
//! - **discard** — it arrived but was unusable: too late for the jitter
//!   buffer, or the buffer overflowed. A *timing* problem, and often
//!   one the receiver can fix by growing its buffer.
//!
//! Reporting a single "loss" figure hides which of those is happening,
//! and they call for opposite responses — send less versus buffer more.

use serde::{Deserialize, Serialize};

/// RFC 3611's recommended gap threshold: the number of consecutive
/// received packets that ends a burst.
pub const DEFAULT_GMIN: u32 = 16;

/// A point-in-time snapshot of one inbound stream.
///
/// Rates are Q8 fixed point — `0..=255` maps to `0.0..=1.0` — matching
/// how RFC 3611 puts loss and discard rates on the wire and how our own
/// `loss_q8` already travels in `FrameAck`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceptionMetrics {
    /// Packets that never arrived, as a fraction of those expected.
    pub loss_rate_q8: u8,
    /// Packets that arrived but were unusable — late, or dropped by an
    /// overflowing buffer.
    pub discard_rate_q8: u8,
    /// Loss+discard density *during burst periods*.
    pub burst_density_q8: u8,
    /// Loss+discard density *during gap periods* — the quiet stretches.
    pub gap_density_q8: u8,
    /// Mean burst length, milliseconds.
    pub burst_duration_ms: u32,
    /// Mean gap length, milliseconds.
    pub gap_duration_ms: u32,
    /// RFC 3550 interarrival jitter, milliseconds.
    pub jitter_ms: u32,
    /// Packets expected so far — the denominator behind every rate.
    pub packets_expected: u64,
    /// Packets actually received.
    pub packets_received: u64,
}

/// Accumulates [`ReceptionMetrics`] from arriving packets.
///
/// One per inbound stream. It holds no clock of its own: the caller
/// passes arrival times, which keeps this testable with a synthetic
/// timeline and free of any I/O.
#[derive(Debug, Clone)]
pub struct ReceptionTracker {
    gmin: u32,
    packet_ms: u32,

    highest_seq: Option<u32>,
    base_seq: Option<u32>,
    received: u64,
    discarded: u64,

    /// RFC 3550 `J(i)`, held in transit-time units and smoothed by 1/16.
    jitter: f64,
    /// Previous packet's transit time (arrival − sender timestamp).
    prev_transit: Option<i64>,

    /// Run of consecutive received packets, for the Gmin test.
    run_length: u32,
    in_burst: bool,
    burst_packets: u64,
    burst_losses: u64,
    gap_packets: u64,
    gap_losses: u64,
    burst_count: u64,
    gap_count: u64,
}

impl ReceptionTracker {
    /// `packet_ms` is the sender's packetisation interval — 20 ms for
    /// our Opus frames. It converts packet counts into the durations
    /// RFC 3611 reports.
    #[must_use]
    pub fn new(packet_ms: u32) -> Self {
        Self::with_gmin(packet_ms, DEFAULT_GMIN)
    }

    #[must_use]
    pub fn with_gmin(packet_ms: u32, gmin: u32) -> Self {
        Self {
            gmin: gmin.max(1),
            packet_ms: packet_ms.max(1),
            highest_seq: None,
            base_seq: None,
            received: 0,
            discarded: 0,
            jitter: 0.0,
            prev_transit: None,
            run_length: 0,
            in_burst: false,
            burst_packets: 0,
            burst_losses: 0,
            gap_packets: 0,
            gap_losses: 0,
            burst_count: 0,
            gap_count: 0,
        }
    }

    /// Record an arriving packet.
    ///
    /// `sender_ms` is the packet's own timestamp; `arrival_ms` is when
    /// we saw it. `usable` is false when the packet arrived but could
    /// not be played — too late, or into a full buffer — which RFC 3611
    /// counts as a *discard* rather than a loss.
    pub fn on_packet(&mut self, sequence: u32, sender_ms: u64, arrival_ms: u64, usable: bool) {
        self.base_seq.get_or_insert(sequence);
        self.received += 1;
        if !usable {
            self.discarded += 1;
        }

        // ── RFC 3550 §6.4.1 interarrival jitter ──────────────────
        //
        //   D(i-1,i) = (Rj - Ri) - (Sj - Si)
        //   J(i) = J(i-1) + (|D(i-1,i)| - J(i-1)) / 16
        //
        // The 1/16 gain is the RFC's, chosen so a single outlier moves
        // the estimate a little and a sustained change moves it a lot.
        // Both are wall-clock milliseconds; the difference is what
        // matters, and it is small. `saturating` rather than `as` so a
        // peer with a wildly wrong clock cannot wrap the arithmetic.
        let transit = i64::try_from(arrival_ms).unwrap_or(i64::MAX)
            - i64::try_from(sender_ms).unwrap_or(i64::MAX);
        if let Some(prev) = self.prev_transit {
            let d =
                f64::from(i32::try_from(transit.saturating_sub(prev).abs()).unwrap_or(i32::MAX));
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.prev_transit = Some(transit);

        // ── Loss detection and the burst/gap walk ────────────────
        //
        // Sequence gaps are the only loss signal available: a packet
        // that never arrives is only visible as a hole.
        let missing = match self.highest_seq {
            Some(prev) => sequence.wrapping_sub(prev).saturating_sub(1),
            None => 0,
        };
        // Reordering shows up as a wrap-sized "gap"; treat anything
        // implausible as reordering rather than inventing thousands of
        // losses. A real outage looks like a modest run of holes.
        let missing = if missing > 1000 { 0 } else { missing };

        for _ in 0..missing {
            self.note_lost();
        }
        self.note_received(usable);

        if self
            .highest_seq
            .is_none_or(|prev| sequence.wrapping_sub(prev) < u32::MAX / 2)
        {
            self.highest_seq = Some(sequence);
        }
    }

    /// A packet we know is gone.
    fn note_lost(&mut self) {
        if !self.in_burst {
            // A loss always opens a burst — RFC 3611's burst starts and
            // ends with a lost or discarded packet.
            self.in_burst = true;
            self.burst_count += 1;
        }
        self.run_length = 0;
        self.burst_packets += 1;
        self.burst_losses += 1;
    }

    /// A packet that arrived, usable or not.
    fn note_received(&mut self, usable: bool) {
        if !usable {
            // A discard counts exactly as a loss for the burst model.
            self.note_lost();
            return;
        }
        if self.in_burst {
            self.run_length += 1;
            self.burst_packets += 1;
            if self.run_length >= self.gmin {
                // `gmin` clean packets in a row ends the burst. Those
                // packets belong to the gap that follows, not the burst
                // they closed, so move them across.
                self.in_burst = false;
                self.gap_count += 1;
                self.burst_packets -= u64::from(self.run_length);
                self.gap_packets += u64::from(self.run_length);
                self.run_length = 0;
            }
        } else {
            self.gap_packets += 1;
        }
    }

    /// Fold in discards the caller counted elsewhere.
    ///
    /// The jitter buffer already tracks its own overflow and late
    /// drops; this lets those be reported without the buffer having to
    /// replay packets through the tracker.
    pub fn note_external_discards(&mut self, count: u64) {
        self.discarded = self.discarded.saturating_add(count);
    }

    /// Current metrics.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "jitter is non-negative and clamped to u32::MAX above"
    )]
    #[must_use]
    pub fn metrics(&self) -> ReceptionMetrics {
        let expected = match (self.base_seq, self.highest_seq) {
            (Some(base), Some(high)) => u64::from(high.wrapping_sub(base)) + 1,
            _ => 0,
        };
        let lost = expected.saturating_sub(self.received);

        ReceptionMetrics {
            loss_rate_q8: rate_q8(lost, expected),
            discard_rate_q8: rate_q8(self.discarded, expected),
            burst_density_q8: rate_q8(self.burst_losses, self.burst_packets),
            gap_density_q8: rate_q8(self.gap_losses, self.gap_packets),
            burst_duration_ms: mean_duration_ms(
                self.burst_packets,
                self.burst_count,
                self.packet_ms,
            ),
            gap_duration_ms: mean_duration_ms(self.gap_packets, self.gap_count, self.packet_ms),
            // Round rather than truncate: sub-millisecond jitter is
            // still jitter, and truncation would report a jittery link
            // as perfectly clean.
            jitter_ms: self.jitter.round().clamp(0.0, f64::from(u32::MAX)) as u32,
            packets_expected: expected,
            packets_received: self.received,
        }
    }
}

/// A count over a total, as Q8. Saturates rather than wrapping, and
/// reports 0 for an empty total — no packets is not 100 % loss.
fn rate_q8(count: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let q8 = count.saturating_mul(255) / total;
    u8::try_from(q8.min(255)).unwrap_or(255)
}

fn mean_duration_ms(packets: u64, periods: u64, packet_ms: u32) -> u32 {
    if periods == 0 {
        return 0;
    }
    let mean_packets = packets / periods;
    u32::try_from(mean_packets.saturating_mul(u64::from(packet_ms))).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `n` packets at a steady 20 ms cadence, skipping any
    /// sequence in `drop`.
    fn run(n: u32, drop: &[u32]) -> ReceptionTracker {
        let mut t = ReceptionTracker::new(20);
        for seq in 0..n {
            if drop.contains(&seq) {
                continue;
            }
            let ms = u64::from(seq) * 20;
            t.on_packet(seq, ms, ms, true);
        }
        t
    }

    #[test]
    fn clean_stream_has_no_loss_and_no_jitter() {
        let m = run(100, &[]).metrics();
        assert_eq!(m.loss_rate_q8, 0);
        assert_eq!(m.jitter_ms, 0);
        assert_eq!(m.packets_received, 100);
        assert_eq!(m.packets_expected, 100);
    }

    /// The point of the whole module: the same loss rate, arriving two
    /// different ways, must not look the same.
    #[test]
    fn bursty_and_uniform_loss_are_distinguishable_at_equal_rate() {
        // 10 lost of 200, spread out. Deliberately not `% 20 == 19`:
        // that drops sequence 199, the last one, so `highest_seq`
        // becomes 198 and the denominator shrinks — the two cases would
        // then differ in loss rate for a reason that has nothing to do
        // with burstiness.
        let uniform: Vec<u32> = (0..200).filter(|s| s % 20 == 10).collect();
        // 10 lost of 200, all together.
        let bursty: Vec<u32> = (100..110).collect();

        let u = run(200, &uniform).metrics();
        let b = run(200, &bursty).metrics();

        assert_eq!(u.loss_rate_q8, b.loss_rate_q8, "same loss rate by design");
        assert!(
            b.burst_duration_ms > u.burst_duration_ms,
            "the clustered outage must show a longer mean burst: {} vs {}",
            b.burst_duration_ms,
            u.burst_duration_ms
        );
        assert!(
            b.burst_density_q8 > u.burst_density_q8,
            "and a denser one: {} vs {}",
            b.burst_density_q8,
            u.burst_density_q8
        );
    }

    #[test]
    fn a_run_of_gmin_clean_packets_closes_the_burst() {
        // One loss, then well over Gmin clean packets, then another.
        let t = run(80, &[10, 60]);
        let m = t.metrics();
        assert_eq!(m.packets_received, 78);
        assert!(
            m.gap_duration_ms > 0,
            "the clean stretch between the two losses is a gap"
        );
    }

    #[test]
    fn discards_are_counted_apart_from_loss() {
        let mut t = ReceptionTracker::new(20);
        for seq in 0..50u32 {
            let ms = u64::from(seq) * 20;
            // Every 10th packet arrives too late to play.
            t.on_packet(seq, ms, ms, seq % 10 != 0);
        }
        let m = t.metrics();
        assert_eq!(m.loss_rate_q8, 0, "nothing was lost — everything arrived");
        assert!(m.discard_rate_q8 > 0, "but some of it was unusable");
    }

    #[test]
    fn jitter_tracks_variable_arrival() {
        let mut t = ReceptionTracker::new(20);
        for seq in 0..100u32 {
            let sent = u64::from(seq) * 20;
            // Alternate a 30 ms extra delay — a 30 ms swing in transit.
            let arrived = sent + if seq % 2 == 0 { 0 } else { 30 };
            t.on_packet(seq, sent, arrived, true);
        }
        let m = t.metrics();
        assert!(
            m.jitter_ms >= 20 && m.jitter_ms <= 30,
            "a 30 ms alternating swing should settle near it, got {}",
            m.jitter_ms
        );
    }

    #[test]
    fn reordering_is_not_reported_as_mass_loss() {
        let mut t = ReceptionTracker::new(20);
        // A late straggler arriving after its successors must not be
        // read as ~4 billion missing packets via wrapping subtraction.
        for seq in [0u32, 1, 2, 5, 3, 4, 6] {
            let ms = u64::from(seq) * 20;
            t.on_packet(seq, ms, ms, true);
        }
        let m = t.metrics();
        assert_eq!(m.packets_received, 7);
        assert!(m.loss_rate_q8 == 0, "nothing was actually lost");
    }

    #[test]
    fn empty_stream_reports_zero_not_total_loss() {
        let m = ReceptionTracker::new(20).metrics();
        assert_eq!(m.loss_rate_q8, 0);
        assert_eq!(m.packets_expected, 0);
    }
}
