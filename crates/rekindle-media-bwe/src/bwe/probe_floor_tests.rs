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
