//! The floor libwebrtc puts under a probe result.

use crate::Bitrate;

/// How far below the acknowledged rate a probe may pull the estimate:
/// "slightly below", to drain a queue we are actually overusing into
/// (libwebrtc `goog_cc_network_control.cc` `kProbeDropThroughputFraction`).
const PROBE_DROP_THROUGHPUT_FRACTION: f64 = 0.85;

/// libwebrtc's probe limit (`goog_cc_network_control.cc`,
/// `limit_probes_lower_than_throughput_estimate_`, on by default): a probe
/// result is raised to at least min(last delay estimate, 0.85 × acked), so
/// a probe that measured a stall cannot drop the estimate below what is
/// being delivered, and one below the current estimate never raises it.
pub(super) fn floor_probe(
    probe: Bitrate,
    last_estimate: Option<Bitrate>,
    acked: Option<Bitrate>,
) -> Bitrate {
    match (last_estimate, acked) {
        (Some(last), Some(acked)) => probe.max(last.min(acked * PROBE_DROP_THROUGHPUT_FRACTION)),
        _ => probe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stalled_probe_cannot_drop_below_delivery() {
        // Call 1 on Mac: a probe read ~0.4 Mbps of burst while ~0.9 Mbps
        // was acknowledged and the estimate stood at 1 Mbps.
        let floored = floor_probe(
            Bitrate::kbps(400),
            Some(Bitrate::kbps(1_000)),
            Some(Bitrate::kbps(900)),
        );
        assert_eq!(floored, Bitrate::kbps(765), "0.85 × acked");
    }

    #[test]
    fn a_low_probe_never_raises_the_estimate() {
        let floored = floor_probe(
            Bitrate::kbps(300),
            Some(Bitrate::kbps(500)),
            Some(Bitrate::kbps(900)),
        );
        assert_eq!(floored, Bitrate::kbps(500), "capped at the last estimate");
    }

    #[test]
    fn a_high_probe_passes() {
        let floored = floor_probe(
            Bitrate::kbps(2_000),
            Some(Bitrate::kbps(500)),
            Some(Bitrate::kbps(400)),
        );
        assert_eq!(floored, Bitrate::kbps(2_000));
    }
}
