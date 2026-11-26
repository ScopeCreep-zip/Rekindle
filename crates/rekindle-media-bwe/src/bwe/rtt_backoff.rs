//! Back off when feedback stops coming: libwebrtc's `RttBasedBackoff`
//! (`modules/congestion_controller/goog_cc/send_side_bandwidth_estimation.cc`),
//! on by default there.
//!
//! The propagation RTT is refreshed by every feedback report. While we keep
//! sending without a new one, the time since the last report is added to
//! it (`CorrectedRtt`), so a silent feedback path reads as a growing RTT.
//! Past 3 s the target is cut to 0.8 of itself, at most once a second, no
//! lower than 5 kbps. RFC 8888 §4: "if multiple consecutive congestion
//! control feedback packets are lost, then the media sender SHOULD rapidly
//! reduce its sending rate".
//!
//! Without it the estimate froze through feedback gaps (call 1: two 5 s
//! windows with no feedback while the video queue grew to 1.9 s).

use std::time::{Duration, Instant};

use super::AckedPacket;
use crate::Bitrate;

/// libwebrtc `configured_limit_` (`WebRTC-Bwe-MaxRttLimit` "limit").
const RTT_LIMIT: Duration = Duration::from_secs(3);
/// libwebrtc `drop_fraction_`.
const DROP_FRACTION: f64 = 0.8;
/// libwebrtc `drop_interval_`.
const DROP_INTERVAL: Duration = Duration::from_secs(1);
/// libwebrtc `bandwidth_floor_`.
const BANDWIDTH_FLOOR: Bitrate = Bitrate::kbps(5);

#[derive(Debug, Default)]
pub(super) struct RttBackoff {
    /// When the propagation RTT was last measured; `None` until the first
    /// feedback, so a route that never had feedback never backs off
    /// (libwebrtc initialises it to plus infinity).
    last_propagation_rtt_update: Option<Instant>,
    last_propagation_rtt: Duration,
    last_packet_sent: Option<Instant>,
    time_last_decrease: Option<Instant>,
}

impl RttBackoff {
    /// A feedback report measured `rtt`: the smallest feedback RTT less the
    /// time each packet waited at the receiver for the report
    /// (`goog_cc_network_control.cc` `min_propagation_rtt`).
    pub(super) fn update_propagation_rtt(&mut self, now: Instant, rtt: Duration) {
        self.last_propagation_rtt_update = Some(now);
        self.last_propagation_rtt = rtt;
    }

    /// A packet left (libwebrtc `OnSentPacket`).
    pub(super) fn on_packet_sent(&mut self, now: Instant) {
        self.last_packet_sent = Some(now);
    }

    /// libwebrtc `CorrectedRtt`: the last propagation RTT plus how long we
    /// have kept sending since it was measured.
    fn corrected_rtt(&self) -> Option<Duration> {
        let updated = self.last_propagation_rtt_update?;
        let sending_since = self.last_packet_sent.map_or(Duration::ZERO, |sent| {
            sent.saturating_duration_since(updated)
        });
        Some(sending_since + self.last_propagation_rtt)
    }

    /// The cut target if the corrected RTT is past the limit and a drop is
    /// due (libwebrtc `SendSideBandwidthEstimation::UpdateEstimate`).
    pub(super) fn backoff(&mut self, now: Instant, current: Bitrate) -> Option<Bitrate> {
        if self.corrected_rtt()? <= RTT_LIMIT {
            return None;
        }
        let due = self
            .time_last_decrease
            .is_none_or(|t| now.saturating_duration_since(t) >= DROP_INTERVAL);
        if !due || current <= BANDWIDTH_FLOOR {
            return None;
        }
        self.time_last_decrease = Some(now);
        Some((current * DROP_FRACTION).max(BANDWIDTH_FLOOR))
    }
}

/// The report's propagation RTT: each packet's feedback RTT less the time it
/// waited at the receiver for the report, the smallest of them
/// (libwebrtc `goog_cc_network_control.cc` `min_propagation_rtt`).
pub(super) fn min_propagation_rtt(acked: &[AckedPacket]) -> Option<Duration> {
    let max_recv = acked.iter().map(|p| p.remote_recv_time).max()?;
    acked
        .iter()
        .map(|p| {
            p.rtt()
                .saturating_sub(max_recv.saturating_duration_since(p.remote_recv_time))
        })
        .min()
}

#[cfg(test)]
#[path = "rtt_backoff_tests.rs"]
mod tests;
