use super::*;

const NO_JITTER: Duration = Duration::ZERO;

fn at(start: Instant, secs: u64) -> Instant {
    start + Duration::from_secs(secs)
}

#[test]
fn a_busy_route_is_never_tested() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start, NO_JITTER);
    for second in (0..600).step_by(5) {
        route.heard(at(start, second));
        assert!(
            !route.should_test(at(start, second), true, || NO_JITTER),
            "tested a busy route at {second} s"
        );
    }
}

#[test]
fn an_idle_route_is_tested_each_period_and_dies_after_three_failures() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start, NO_JITTER);
    assert!(!route.should_test(at(start, 39), false, || NO_JITTER));
    assert!(route.should_test(at(start, 40), false, || NO_JITTER));
    assert_eq!(
        route.tested(at(start, 55), false, NO_JITTER, NO_JITTER),
        SelfTest::Retry
    );
    // The retest is 10 s (plus its jitter) after the failure.
    assert!(!route.should_test(at(start, 64), false, || NO_JITTER));
    assert!(route.should_test(at(start, 65), false, || NO_JITTER));
    assert_eq!(
        route.tested(at(start, 80), false, NO_JITTER, NO_JITTER),
        SelfTest::Retry
    );
    assert!(route.should_test(at(start, 90), false, || NO_JITTER));
    assert_eq!(
        route.tested(at(start, 105), false, NO_JITTER, NO_JITTER),
        SelfTest::Dead
    );
}

#[test]
fn an_answer_resets_the_failures() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start, NO_JITTER);
    for round in 0..5 {
        let now = at(start, 100 * round);
        assert_eq!(
            route.tested(now, false, NO_JITTER, NO_JITTER),
            SelfTest::Retry
        );
        assert_eq!(
            route.tested(now, true, NO_JITTER, NO_JITTER),
            SelfTest::Healthy
        );
    }
}

#[test]
fn silence_while_peers_should_send_tests_at_once_and_not_again_for_120_s() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start, NO_JITTER);
    route.heard(at(start, 100));
    // Heard 10 s ago: the period's test is skipped (the idle gate).
    assert!(!route.should_test(at(start, 110), false, || NO_JITTER));
    // A passing self-test is not a peer's traffic: it does not end silence.
    route.tested(at(start, 200), true, NO_JITTER, NO_JITTER);
    assert!(!route.should_test(at(start, 219), true, || NO_JITTER));
    assert!(route.should_test(at(start, 220), true, || NO_JITTER));
    route.tested(at(start, 221), true, NO_JITTER, NO_JITTER);
    assert!(!route.should_test(at(start, 250), true, || NO_JITTER));
}

#[test]
fn a_network_change_tests_at_once_and_one_failure_is_death() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start, NO_JITTER);
    route.heard(at(start, 10));
    route.network_changed(at(start, 11));
    assert!(route.should_test(at(start, 11), false, || NO_JITTER));
    assert_eq!(
        route.tested(at(start, 12), false, NO_JITTER, NO_JITTER),
        SelfTest::Dead
    );
}

#[test]
fn republishes_are_debounced_and_at_least_60_s_apart() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start, NO_JITTER);
    assert_eq!(
        route.publish_at(start),
        start,
        "the first publish goes at once"
    );
    route.published(at(start, 3));
    assert_eq!(route.publish_at(at(start, 10)), at(start, 63));
    assert_eq!(route.publish_at(at(start, 100)), at(start, 103));
    route.route_replaced(at(start, 100), NO_JITTER);
    assert_eq!(
        route.publish_at(at(start, 10)),
        at(start, 63),
        "pacing outlives a replacement"
    );
}

#[test]
fn eight_unacked_sends_or_one_ack_deadline_make_a_route_suspect() {
    let start = Instant::now();
    let mut peer = PeerLiveness::default();
    for _ in 0..7 {
        peer.sent(start);
    }
    assert!(!peer.suspect(start));
    peer.sent(start);
    assert!(peer.suspect(start));
    assert_eq!(peer.before_send(start), Suspicion::Refetch);

    let mut new = PeerLiveness::default();
    new.sent(start);
    assert!(!new.suspect(at(start, 54)));
    assert!(new.suspect(at(start, 55)));
    let mut proven = PeerLiveness::default();
    proven.heard();
    proven.sent(start);
    assert!(!proven.suspect(at(start, 29)));
    assert!(proven.suspect(at(start, 30)));
}

#[test]
fn a_refetch_runs_at_most_once_per_60_s_and_takes_only_a_higher_seq() {
    let start = Instant::now();
    let mut peer = PeerLiveness::default();
    assert_eq!(peer.observe(4), Fetched::Newer);
    for _ in 0..8 {
        peer.sent(start);
    }
    assert!(!peer.begin_round(start, Some(b"old".to_vec())));
    assert!(peer.failure_cached(at(start, 119), b"old"));
    assert!(!peer.failure_cached(at(start, 120), b"old"));
    assert!(!peer.failure_cached(at(start, 1), b"new"));
    assert_eq!(peer.observe(4), Fetched::Same);
    assert_eq!(peer.observe(3), Fetched::Stale);
    assert_eq!(peer.observe(5), Fetched::Newer);
    assert_eq!(
        peer.observe(4),
        Fetched::Stale,
        "never back to an older record"
    );
    for _ in 0..8 {
        peer.sent(at(start, 10));
    }
    assert_eq!(peer.before_send(at(start, 59)), Suspicion::None);
    assert_eq!(peer.before_send(at(start, 60)), Suspicion::Refetch);
    peer.clear_failure();
    assert!(!peer.failure_cached(at(start, 61), b"old"));
}

#[test]
fn six_rounds_make_the_peer_unreachable_and_hearing_it_resets() {
    let start = Instant::now();
    let mut peer = PeerLiveness::default();
    for round in 1..=6 {
        let became = peer.begin_round(at(start, 60 * round), None);
        assert_eq!(became, round == 6);
    }
    assert!(peer.unreachable());
    let now = at(start, 360);
    assert!(!peer.fast_path_allowed(now));
    assert!(peer.fast_path_allowed(at(start, 420)));
    // The next step is 1.2 min later.
    assert!(!peer.fast_path_allowed(at(start, 491)));
    assert!(peer.fast_path_allowed(at(start, 492)));
    assert!(peer.heard());
    assert!(!peer.unreachable());
    assert!(peer.fast_path_allowed(at(start, 493)));
}
