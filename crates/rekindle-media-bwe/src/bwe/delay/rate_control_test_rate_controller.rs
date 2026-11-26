use std::time::Instant;

use super::{duration_ms, RateControl, Signal};

fn make_control(estimated_bitrate: u64) -> RateControl {
    RateControl::new(estimated_bitrate.into(), 10_000.into(), 50_000_000.into())
}

#[test]
fn near_max_increase_follows_libwebrtc() {
    // 500 kbps at 30 fps: 16,667-bit frames in 2 packets of 8,333
    // bits; RTT 300 ms gives a response time of (300 + 100) · 2 =
    // 800 ms: 10.4 kbps/s.
    let mut rate_controller = make_control(500_000);
    rate_controller.last_rtt = Some(duration_ms(300));
    let rate = rate_controller.near_max_increase_bps_per_s();
    assert!((rate - 10_416.7).abs() < 1.0, "{rate}");
    // At low rates the 4 kbps/s floor holds.
    let mut low = make_control(20_000);
    low.last_rtt = Some(duration_ms(300));
    assert!((low.near_max_increase_bps_per_s() - 4_000.0).abs() < f64::EPSILON);
}

#[test]
fn test_initial_estimate() {
    let rate_controller = make_control(100_000);

    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 100_000);
}

#[test]
fn test_normal_yields_multiplicative_increase() {
    let now = Instant::now();
    let mut rate_controller = make_control(100_000);
    // Seed last estimate value
    rate_controller.update(Signal::Normal, 85_000.into(), None, now);
    assert_eq!(
        rate_controller.estimated_bitrate().as_u64(),
        101_000,
        "Initial estimate should increase by the minimum(1Kbit/s)"
    );

    rate_controller.update(Signal::Normal, 95_000.into(), None, now + duration_ms(500));
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 104_963);

    rate_controller.update(Signal::Normal, 97_000.into(), None, now + duration_ms(1000));
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 109_081);
}

#[test]
fn test_normal_to_under_use_yields_hold() {
    let now = Instant::now();
    let mut rate_controller = make_control(100_000);
    // Seed last estimate value
    rate_controller.update(Signal::Normal, 85_000.into(), None, now);
    assert_eq!(
        rate_controller.estimated_bitrate().as_u64(),
        101_000,
        "Initial estimate should increase by the minimum(1Kbit/s)"
    );

    // Should remain in increase and increase estimate
    rate_controller.update(Signal::Normal, 95_000.into(), None, now + duration_ms(500));
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 104_963);

    // Should transition to hold
    rate_controller.update(
        Signal::Underuse,
        97_000.into(),
        None,
        now + duration_ms(1000),
    );
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 104_963);

    // Should remain in hold and not modify estimates
    rate_controller.update(
        Signal::Underuse,
        97_000.into(),
        None,
        now + duration_ms(2000),
    );
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 104_963);
}

#[test]
fn test_immediate_overuse() {
    let now = Instant::now();
    let mut rate_controller = make_control(100_000);
    // Seed last estimate value
    rate_controller.update(Signal::Normal, 85_000.into(), None, now);

    rate_controller.update(Signal::Overuse, 90_000.into(), None, now + duration_ms(500));
    assert_eq!(
        rate_controller.estimated_bitrate().as_u64(),
        76_500,
        "When overuse is detected we should reduce the estimate to \
                85% of the obeserved rate immediately"
    );
}

#[test]
fn test_immediate_overuse_then_stable() {
    let now = Instant::now();
    let mut rate_controller = make_control(100_000);
    // Seed last estimate value
    rate_controller.update(Signal::Normal, 85_000.into(), Some(duration_ms(80)), now);

    rate_controller.update(Signal::Overuse, 90_000.into(), None, now + duration_ms(500));
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 76_500);

    rate_controller.update(
        Signal::Overuse,
        75_000.into(),
        None,
        now + duration_ms(1000),
    );
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 63_750);

    rate_controller.update(Signal::Normal, 60_000.into(), None, now + duration_ms(1500));
    // NB: This matches libWebRTC but diverges from the spec
    assert_eq!(
        rate_controller.estimated_bitrate().as_u64(),
        66_251,
        "After adjusting on overuse we immediately return to increase on the next normal signal"
    );

    rate_controller.update(Signal::Normal, 60_000.into(), None, now + duration_ms(2500));
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 71_552,);

    // NB: Additive increase because we are nearing convergence. At
    // 71,552 bps a 30 fps frame is 2,385 bits (one packet); RTT
    // 80 ms gives a 360 ms response time: 6,625 bps/s over 1 s
    // (libwebrtc `AdditiveRateIncrease`).
    rate_controller.update(Signal::Normal, 70_000.into(), None, now + duration_ms(3500));
    assert_eq!(rate_controller.estimated_bitrate().as_u64(), 78_177);
}
