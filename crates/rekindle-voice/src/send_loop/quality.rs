//! Receiver-report intake and the 5-second quality pass.
//!
//! This is the send loop's *feedback* half: where it learns what its
//! audio looks like at the far end, and acts on it.
//!
//! Before receiver reports existed the loop's only quality signal was
//! `send_failures / packets_sent` — a count of our own local send
//! errors. That cannot observe loss (the `send()` succeeded), cannot
//! observe jitter or round trip, and reads a link dropping a fifth of
//! its packets as a clean 0 %. Every media decision downstream of it —
//! Opus FEC sizing, the bitrate ladder — was therefore tuned against a
//! number that did not measure the thing being tuned.
//!
//! The fallback is still here, used only when no peer has reported, and
//! it is a fallback rather than a default for that reason.

use std::time::{Duration, Instant};

use rekindle_media_stats::{LinkState, LinkTracker, QualityScore, ReceptionMetrics};

use super::VoiceSendLoop;
use crate::codec::{DEFAULT_BITRATE_BPS, MIN_BITRATE_BPS};
use crate::receiver_report::VoiceReceiverReport;
use crate::session_deps::{SendLinkStats, VoiceSessionEvent};

/// Q8 rate as a whole percent, for log lines a human reads live.
fn q8_pct(q8: u8) -> u32 {
    u32::from(q8) * 100 / 255
}

/// Rank for picking the worst link. `Recovering` outranks `Fair`
/// because it demands action (the far end's decoder has a hole), not
/// because the R factor behind it is lower.
fn link_severity(state: LinkState) -> u8 {
    match state {
        LinkState::Good => 0,
        LinkState::Fair => 1,
        LinkState::Recovering => 2,
        LinkState::Poor => 3,
        LinkState::Lost => 4,
    }
}

/// The `ConnectionQuality` event's wire vocabulary. `Lost` and
/// `Recovering` are reported distinctly rather than collapsed into
/// "poor": a user needs to see the difference between a call that is
/// degraded and one that has dropped and is coming back.
fn link_label(state: LinkState) -> &'static str {
    match state {
        LinkState::Good => "good",
        LinkState::Fair => "fair",
        LinkState::Poor => "poor",
        LinkState::Lost => "lost",
        LinkState::Recovering => "recovering",
    }
}

/// One peer's view of our outbound stream, from their reports.
pub(super) struct PeerLink {
    /// Link state advanced per report — the state machine the loop
    /// branches on, rather than a bare threshold on the latest rate.
    tracker: LinkTracker,
    /// Metrics from the newest report.
    metrics: ReceptionMetrics,
    /// `packets_received` from the previous report. Reports carry
    /// session totals, so the window count — which is what
    /// [`LinkTracker`]'s silence detection needs — is the difference.
    last_packets_received: u64,
    /// Round trip from the newest report, when it was computable.
    rtt_ms: Option<u32>,
    /// The peer's playout depth, for the one-way delay estimate.
    jb_nominal_ms: u32,
    /// Score from the newest report — kept so the 5 s quality pass can
    /// report the same verdict it acted on, rather than recomputing
    /// from a staler window.
    score: QualityScore,
    /// Wall-clock arrival of the previous report from this peer. Reports
    /// are generated on a fixed cadence at the far end, so the gap
    /// between arrivals here measures the RETURN path (B→A): a gap much
    /// larger than the cadence means our reports are themselves delayed,
    /// which inflates RTT independently of how promptly our audio
    /// reached the peer. Part of the Phase 1 RTT decomposition.
    last_report_at: Option<std::time::Instant>,
}

impl VoiceSendLoop {
    /// Fold one verified receiver report into that peer's link view.
    ///
    /// The signature was already checked by
    /// [`VoiceReceiverReport::from_wire`] at the dispatch boundary, so
    /// the metrics here are attributable to `reporter_key`.
    pub(super) fn note_receiver_report(&mut self, report: &VoiceReceiverReport) {
        let peer = hex::encode(&report.reporter_key);
        let rtt_ms = report.round_trip_ms(rekindle_utils::timestamp_ms());
        let link = self
            .peer_links
            .entry(peer.clone())
            .or_insert_with(|| PeerLink {
                tracker: LinkTracker::default(),
                metrics: ReceptionMetrics::default(),
                last_packets_received: 0,
                rtt_ms: None,
                jb_nominal_ms: 0,
                score: QualityScore {
                    r_factor: 0,
                    mos_lq: 0.0,
                    mos_cq: 0.0,
                    state: LinkState::Good,
                },
                last_report_at: None,
            });

        // RTT decomposition (Phase 1): `rtt = (now − lsr) − dlsr`.
        //   lsr_age = now − lsr  → total elapsed since we sent the packet
        //     the peer echoed = the A→B audio delay (our clock domain,
        //     since lsr is our own packet timestamp echoed back).
        //   dlsr                 → the peer's hold before reporting.
        //   report_gap           → wall gap since this peer's last report
        //     = the B→A return-path delay (reports have a fixed cadence).
        // Together these say which leg inflates a large RTT, so we stop
        // guessing whether the 8–16 s figure is outbound audio, the
        // peer's buffer, or a congested report path.
        let now_ms = rekindle_utils::timestamp_ms();
        // Media-plane proof of life for the presence reconcile: a
        // verified report proves this peer is alive even while VAD
        // keeps them silent (no voice packets for the receive loop to
        // note). Same wall clock the receive loop's note site stamps.
        self.media_liveness.note(&peer, now_ms);
        let lsr_age_ms = now_ms.saturating_sub(report.lsr_ms);
        let report_gap_ms = link
            .last_report_at
            .map(|t| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX));
        link.last_report_at = Some(std::time::Instant::now());

        // Reports carry session totals; the tracker wants this window.
        // `saturating_sub` covers a peer that restarted its session and
        // began counting again from zero.
        let window_packets = report
            .metrics
            .packets_received
            .saturating_sub(link.last_packets_received);
        link.last_packets_received = report.metrics.packets_received;
        link.metrics = report.metrics;
        link.jb_nominal_ms = report.jb_nominal_ms;
        if rtt_ms.is_some() {
            link.rtt_ms = rtt_ms;
        }

        // One-way delay for the E-model: half the round trip plus the
        // depth the far end is holding before playout. Both are real
        // contributions to mouth-to-ear, and the jitter buffer's is
        // often the larger of the two.
        let one_way = rtt_ms.or(link.rtt_ms).unwrap_or(0) / 2 + report.jb_nominal_ms;
        let score = rekindle_media_stats::score(
            &report.metrics,
            one_way,
            window_packets,
            &mut link.tracker,
        );
        link.score = score;
        // `info!`, not `debug!`: one line per peer per 5 s is the entire
        // record of what a real call measured, and the default filter is
        // `info`. Wanting it means editing a filter on a remote machine
        // mid-call, which is exactly when you cannot. Percentages as
        // well as Q8 so the line is readable without doing the /255 in
        // your head. `voice link` is the grep handle.
        tracing::info!(
            peer = %peer,
            loss_pct = q8_pct(report.metrics.loss_rate_q8),
            discard_pct = q8_pct(report.metrics.discard_rate_q8),
            burst_pct = q8_pct(report.metrics.burst_density_q8),
            jitter_ms = report.metrics.jitter_ms,
            rtt_ms = ?rtt_ms,
            lsr_age_ms,
            dlsr_ms = report.dlsr_ms,
            report_gap_ms = ?report_gap_ms,
            jb_nominal_ms = report.jb_nominal_ms,
            one_way_ms = one_way,
            r_factor = score.r_factor,
            mos_lq = score.mos_lq,
            mos_cq = score.mos_cq,
            state = ?score.state,
            window_packets,
            expected = report.metrics.packets_expected,
            received = report.metrics.packets_received,
            "voice link"
        );
    }

    /// The peer whose view of our stream is worst.
    ///
    /// A mesh sender encodes **one** stream for every peer, so its
    /// bitrate and FEC have to satisfy the worst receiver — tuning to
    /// the average would leave that peer permanently broken.
    ///
    /// `bitrate_bps` is left at 0 here; the caller stamps it once the
    /// bitrate decision is made.
    fn worst_link(&self) -> Option<SendLinkStats> {
        self.peer_links
            .values()
            .map(|l| SendLinkStats {
                metrics: l.metrics,
                score: l.score,
                rtt_ms: l.rtt_ms,
                bitrate_bps: 0,
            })
            .max_by_key(|s| (link_severity(s.score.state), s.metrics.loss_rate_q8))
    }

    pub(super) fn report_quality_if_due(&mut self) {
        if self.last_quality_report.elapsed() < Duration::from_secs(5) {
            return;
        }

        // Loss as the far end actually measured it, when any peer has
        // reported. Falling back to `send_failures / packets_sent` is
        // strictly a last resort: that ratio counts our own local send
        // errors, and a `send()` that succeeds says nothing about
        // whether the packet arrived — so on a link dropping a fifth of
        // its packets it reads a clean 0 %.
        let worst = self.worst_link();
        let (quality, loss_pct_u32) = if let Some(w) = worst.as_ref() {
            (
                link_label(w.score.state),
                u32::from(w.metrics.loss_rate_q8.max(w.metrics.discard_rate_q8)) * 100 / 255,
            )
        } else {
            let local = self
                .send_failures
                .saturating_mul(100)
                .checked_div(self.packets_sent)
                .and_then(|loss| u32::try_from(loss).ok())
                .unwrap_or(0);
            let label = match local {
                0..5 => "good",
                5..15 => "fair",
                _ => "poor",
            };
            (label, local)
        };
        // Opus in-band FEC is sized from the receiver's loss, which is
        // the number it was always supposed to use: FEC exists to
        // reconstruct what the *network* dropped, and only the receiver
        // can see that.
        let loss_i32 = i32::try_from(loss_pct_u32.min(100)).unwrap_or(100);
        let _ = self.codec.set_packet_loss_perc(loss_i32);

        // Bitrate: group size sets the baseline (a mesh sender pays it
        // once per peer), then a struggling link pulls it down. Cannot
        // hold the tokio Mutex synchronously, so use try_lock.
        // A mesh sender pays the bitrate once per peer, so the ladder
        // trades per-stream quality against total egress as the roster
        // grows. Rungs are relative to the codec default rather than
        // spelled out, so raising that raises the whole ladder and the
        // two cannot drift.
        let peer_count = self.transport.try_lock().map(|t| t.peer_count()).ok();
        let baseline = match peer_count {
            // DM, or a mesh small enough to afford full rate.
            Some(0..=2) | None => DEFAULT_BITRATE_BPS,
            // Still full mesh (the topology switches to an SFU above
            // four), so egress is the binding constraint here.
            Some(3..=7) => DEFAULT_BITRATE_BPS * 3 / 4,
            Some(_) => DEFAULT_BITRATE_BPS / 2,
        };
        // Backing off on a Poor link trades clarity for arrival: fewer
        // bits per packet means smaller packets, which a congested path
        // is likelier to deliver. `Lost` holds the floor rather than
        // dropping further — there is nothing left to concede, and it
        // has to be able to recover.
        let target_bps = match worst.as_ref().map(|w| w.score.state) {
            Some(LinkState::Poor | LinkState::Lost) => (baseline * 2 / 3).max(MIN_BITRATE_BPS),
            Some(LinkState::Fair | LinkState::Recovering) => {
                (baseline * 5 / 6).max(MIN_BITRATE_BPS)
            }
            Some(LinkState::Good) | None => baseline,
        };
        if peer_count.is_some() {
            let _ = self.codec.set_bitrate(target_bps);
        }

        // Emitted after the decisions above so the event carries the
        // action taken, not just the measurement behind it.
        self.deps
            .emit_voice_event(VoiceSessionEvent::ConnectionQuality {
                quality: quality.to_string(),
                link: worst.map(|mut w| {
                    w.bitrate_bps = u32::try_from(target_bps).unwrap_or(0);
                    w
                }),
            });

        self.packets_sent = 0;
        self.send_failures = 0;
        self.last_quality_report = Instant::now();
    }
}
