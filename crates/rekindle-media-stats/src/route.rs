//! Route delay measurement for a call (plan E4.3.0, D27).
//!
//! Every media constant in the stack — jitter-buffer bounds, video rate
//! ceilings, the latency budget — was set without a measured route delay
//! (`r6-media-engines.md` §6: "no measured distribution of route one-way
//! delay or jitter"). This module turns each receiver report into an
//! estimate of the route's one-way delay and keeps the call-long
//! distribution, so the numbers exist before any engine is retuned.
//!
//! **One-way delay.** Clocks are not synchronised, so one-way delay is
//! estimated as the route floor plus the queueing above it:
//! `rtt / 2 + relative delay`. The relative delay is each packet's transit
//! above the window minimum, which the receiver measures and the clock
//! offset cancels out of ([`crate::ReceptionMetrics::delay_p95_ms`]). The
//! `rtt / 2` floor assumes a symmetric route at its least-loaded packet —
//! the same assumption RTCP's NTP-style offset estimate makes (RFC 3550
//! §6.4.1, LSR/DLSR).
//!
//! **Mouth-to-ear.** One-way p95 plus the far end's playout depth plus the
//! fixed in-process stages, modelled at [`IN_PROCESS_DELAY_MS`]. Above
//! ITU-T G.114's 400 ms limit the call is flagged.

/// Fixed in-process delay: capture buffering, the Opus frame and
/// lookahead, audio processing, send queue, decode and playout — the
/// "≈ 75–95 ms" total of the r6 §7.2 budget, at its midpoint. A model, to
/// be replaced by measurement.
pub const IN_PROCESS_DELAY_MS: u32 = 85;

/// ITU-T G.114's one-way mouth-to-ear limit for conversational voice.
pub const G114_LIMIT_MS: u32 = 400;

/// Windows a call keeps (one per receiver report, ~5 s): three hours.
/// Older windows are forgotten so a long call stays bounded.
const MAX_WINDOWS: usize = 2_160;

/// One receiver report's route estimate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RouteEstimate {
    /// Estimated one-way route delay, median, ms.
    pub one_way_p50_ms: u32,
    /// Estimated one-way route delay, 95th percentile, ms.
    pub one_way_p95_ms: u32,
    /// One-way p95 + far-end playout depth + in-process stages, ms.
    pub mouth_to_ear_ms: u32,
    /// Mouth-to-ear above [`G114_LIMIT_MS`].
    pub outside_g114: bool,
}

/// The estimate one report gives: `rtt / 2` plus the receiver's relative
/// delay percentiles, and the mouth-to-ear that implies at the far end's
/// playout depth `jb_nominal_ms`.
#[must_use]
pub fn route_estimate(
    rtt_ms: u32,
    delay_p50_ms: u32,
    delay_p95_ms: u32,
    jb_nominal_ms: u32,
) -> RouteEstimate {
    let floor = rtt_ms / 2;
    let one_way_p95_ms = floor.saturating_add(delay_p95_ms);
    let mouth_to_ear_ms = one_way_p95_ms
        .saturating_add(jb_nominal_ms)
        .saturating_add(IN_PROCESS_DELAY_MS);
    RouteEstimate {
        one_way_p50_ms: floor.saturating_add(delay_p50_ms),
        one_way_p95_ms,
        mouth_to_ear_ms,
        outside_g114: mouth_to_ear_ms > G114_LIMIT_MS,
    }
}

/// One window's figures.
#[derive(Debug, Clone, Copy)]
struct Window {
    estimate: RouteEstimate,
    jitter_ms: u32,
}

/// A call's route measurement for one peer: every window's estimate, for
/// the distribution over the call.
#[derive(Debug, Clone, Default)]
pub struct RouteHistory {
    windows: std::collections::VecDeque<Window>,
    /// Latest loss rate and mean burst length (both already session-long
    /// in the report).
    loss_rate_q8: u8,
    burst_duration_ms: u32,
}

/// The distribution over a call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RouteSummary {
    /// Windows measured.
    pub windows: usize,
    /// Median of the windows' one-way medians, ms.
    pub one_way_p50_ms: u32,
    /// 95th percentile of the windows' one-way p95s, ms.
    pub one_way_p95_ms: u32,
    /// 95th percentile of the windows' RFC 3550 jitter, ms.
    pub jitter_p95_ms: u32,
    /// 95th percentile of the windows' mouth-to-ear, ms.
    pub mouth_to_ear_p95_ms: u32,
    /// Loss rate over the call, Q8.
    pub loss_rate_q8: u8,
    /// Mean loss burst length over the call, ms (RFC 3611).
    pub burst_duration_ms: u32,
}

impl RouteHistory {
    /// Fold one report's window in.
    pub fn note(
        &mut self,
        estimate: RouteEstimate,
        jitter_ms: u32,
        loss_rate_q8: u8,
        burst_duration_ms: u32,
    ) {
        if self.windows.len() == MAX_WINDOWS {
            self.windows.pop_front();
        }
        self.windows.push_back(Window {
            estimate,
            jitter_ms,
        });
        self.loss_rate_q8 = loss_rate_q8;
        self.burst_duration_ms = burst_duration_ms;
    }

    /// The distribution so far; all zero before the first window.
    #[must_use]
    pub fn summary(&self) -> RouteSummary {
        let pick = |pct: usize, f: fn(&Window) -> u32| {
            percentile(self.windows.iter().map(f).collect(), pct)
        };
        RouteSummary {
            windows: self.windows.len(),
            one_way_p50_ms: pick(50, |w| w.estimate.one_way_p50_ms),
            one_way_p95_ms: pick(95, |w| w.estimate.one_way_p95_ms),
            jitter_p95_ms: pick(95, |w| w.jitter_ms),
            mouth_to_ear_p95_ms: pick(95, |w| w.estimate.mouth_to_ear_ms),
            loss_rate_q8: self.loss_rate_q8,
            burst_duration_ms: self.burst_duration_ms,
        }
    }
}

/// Nearest-rank percentile; 0 for no values.
fn percentile(mut values: Vec<u32>, pct: usize) -> u32 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let rank = (values.len() * pct).div_ceil(100).max(1) - 1;
    values[rank.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_is_half_rtt_plus_queueing() {
        let e = route_estimate(200, 10, 60, 80);
        assert_eq!(e.one_way_p50_ms, 110);
        assert_eq!(e.one_way_p95_ms, 160);
        assert_eq!(e.mouth_to_ear_ms, 160 + 80 + IN_PROCESS_DELAY_MS);
        assert!(!e.outside_g114);
    }

    #[test]
    fn a_slow_route_is_outside_g114() {
        let e = route_estimate(500, 20, 120, 120);
        assert!(e.mouth_to_ear_ms > G114_LIMIT_MS);
        assert!(e.outside_g114);
    }

    #[test]
    fn summary_takes_percentiles_over_windows() {
        let mut h = RouteHistory::default();
        assert_eq!(h.summary(), RouteSummary::default());
        for i in 0..20u32 {
            // One slow window in twenty.
            let rtt = if i == 19 { 1_000 } else { 200 };
            h.note(route_estimate(rtt, 0, 0, 40), i, 3, 40);
        }
        let s = h.summary();
        assert_eq!(s.windows, 20);
        assert_eq!(s.one_way_p50_ms, 100);
        assert_eq!(
            s.one_way_p95_ms, 100,
            "one slow window is 5 %: at, not under, p95"
        );
        assert_eq!(s.jitter_p95_ms, 18);
        assert_eq!(s.loss_rate_q8, 3);
        assert_eq!(s.burst_duration_ms, 40);
    }
}
