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
