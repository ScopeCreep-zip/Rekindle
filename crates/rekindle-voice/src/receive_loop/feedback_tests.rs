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
