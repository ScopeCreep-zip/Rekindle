//! Transport feedback emission (plans E4.3.2, E4.3.3): one signed RFC
//! 8888-shaped report per peer whose media arrived, sent back over that
//! peer's route.
//!
//! The interval is libwebrtc's
//! (`transport_sequence_number_feedback_generator.cc`): 100 ms until the
//! route has an estimate, then whatever keeps reports at 5 % of our own
//! send estimate toward that peer, the path the reports travel, within
//! 50–250 ms. A report's size is what the route carries for it, Veilid's
//! per-message cost included, so on a Veilid route the interval sits near
//! its 250 ms ceiling until the estimate is several Mbps.

use std::time::{Duration, Instant};

use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use rekindle_codec::capnp_codec::SignedWire;

use super::VoiceReceiveLoop;
use crate::media_frame::FEEDBACK_TAG;
use crate::transport::egress::ROUTE_OVERHEAD_BYTES;

const DEFAULT_INTERVAL: Duration = Duration::from_millis(100);
const MIN_INTERVAL: Duration = Duration::from_millis(50);
const MAX_INTERVAL: Duration = Duration::from_millis(250);

/// The interval at which reports of `report_bytes` on the wire take 5 %
/// of `estimate_bps` (libwebrtc `OnSendBandwidthEstimateChanged`).
pub(super) fn feedback_interval(estimate_bps: Option<u64>, report_bytes: usize) -> Duration {
    let Some(estimate_bps) = estimate_bps else {
        return DEFAULT_INTERVAL;
    };
    let report_bits = f64::from(u32::try_from(report_bytes).unwrap_or(u32::MAX)) * 8.0;
    let budget_bps = 0.05 * f64::from(u32::try_from(estimate_bps).unwrap_or(u32::MAX));
    let min_budget_bps = report_bits / MAX_INTERVAL.as_secs_f64();
    if budget_bps <= min_budget_bps {
        MAX_INTERVAL
    } else {
        Duration::from_secs_f64(report_bits / budget_bps).max(MIN_INTERVAL)
    }
}

impl VoiceReceiveLoop {
    /// Send each peer its report when its interval has passed. Without a
    /// signing identity no report would be accepted, so none is built.
    pub(super) fn send_feedback_if_due(&mut self) {
        let now = Instant::now();
        let Some(signing_key) = self.report_signing_key.clone() else {
            return;
        };
        let report_time_ms = self.arrivals.report_time_ms(now);
        for peer in self.arrivals.peers_with_arrivals() {
            let (last, report_bytes) = self
                .feedback_sent
                .get(&peer)
                .copied()
                .unwrap_or((None, ROUTE_OVERHEAD_BYTES));
            let estimate = self.allocator.route(&peer).map(|r| r.estimate_bps);
            let interval = feedback_interval(estimate, report_bytes);
            if last.is_some_and(|t| now.saturating_duration_since(t) < interval) {
                continue;
            }
            let Some(body) = self.arrivals.take_report(&peer, now) else {
                continue;
            };
            self.feedback_stats.note_built(&peer);
            let mut feedback = TransportFeedback {
                reporter_key: self.our_key_bytes.clone(),
                begin_seq: body.begin_seq,
                report_time_ms,
                arrivals: body.arrivals,
                sig: Vec::new(),
            };
            feedback.sign(&signing_key);
            let encoded = feedback.encode();
            let mut wire = Vec::with_capacity(1 + encoded.len());
            wire.push(FEEDBACK_TAG);
            wire.extend_from_slice(&encoded);
            self.feedback_sent
                .insert(peer.clone(), (Some(now), wire.len() + ROUTE_OVERHEAD_BYTES));
            self.deps.send_receiver_report(&peer, wire);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_follows_five_percent_of_the_estimate() {
        assert_eq!(feedback_interval(None, 1_800), DEFAULT_INTERVAL);
        // 1800 B every 250 ms is 57.6 kbps: 5 % of 1.152 Mbps.
        assert_eq!(feedback_interval(Some(1_000_000), 1_800), MAX_INTERVAL);
        let at_4m = feedback_interval(Some(4_000_000), 1_800);
        assert_eq!(at_4m, Duration::from_secs_f64(14_400.0 / 200_000.0));
        assert_eq!(feedback_interval(Some(100_000_000), 1_800), MIN_INTERVAL);
    }
}
