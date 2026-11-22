use super::*;

fn at(start: Instant, secs: u64) -> Instant {
    start + Duration::from_secs(secs)
}

/// Plan RV1: silence rotates the General route only while a peer should be
/// answering, and only after 120 s with nothing over the route.
#[test]
fn silence_rotates_only_while_a_peer_should_answer() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start);
    assert!(
        !route.silent(at(start, 119), true),
        "younger than the trigger"
    );
    assert!(route.silent(at(start, 120), true));
    assert!(
        !route.silent(at(start, 600), false),
        "no peer should answer"
    );
    route.heard(at(start, 200));
    assert!(!route.silent(at(start, 300), true), "inbound arrived");
    assert!(route.silent(at(start, 320), true));
    route.route_replaced(at(start, 320));
    assert!(
        !route.silent(at(start, 400), true),
        "a fresh route gets the full trigger"
    );
}

/// Plan RV1: a peer's report rotates a route, unless the route is younger
/// than one re-fetch gap (the report is about the route it replaced).
#[test]
fn a_report_rotates_a_route_older_than_a_refetch_gap() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start);
    assert!(!route.reported(at(start, 59)));
    assert!(route.reported(at(start, 60)));
    route.route_replaced(at(start, 60));
    assert!(!route.reported(at(start, 90)), "about the replaced route");
    assert!(route.reported(at(start, 120)));
}

#[test]
fn republishes_are_debounced_and_at_least_60_s_apart() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start);
    assert_eq!(
        route.publish_at(start),
        start,
        "the first publish goes at once"
    );
    route.published(at(start, 3));
    assert_eq!(route.publish_at(at(start, 10)), at(start, 63));
    assert_eq!(route.publish_at(at(start, 100)), at(start, 103));
    route.route_replaced(at(start, 100));
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

/// Plan RV1, the third evidence source: a friend's chain entry whose hint
/// never arrives within the window rotates the route; a hint in time, or a
/// route too young for the friend to hold, does not.
#[test]
fn a_chain_ahead_of_its_hints_rotates() {
    let start = Instant::now();
    let mut rules = InboundLiveness::new(start);
    let old = at(start, REPORT_ROTATE_AGE.as_secs());
    rules.chain_changed("friend", old);
    assert!(!rules.chain_ahead(old), "the hint may still come");
    rules.hinted("friend", old + Duration::from_secs(5));
    assert!(!rules.chain_ahead(old + HINT_WINDOW), "the hint came");
    let later = old + HINT_WINDOW;
    rules.chain_changed("friend", later + Duration::from_secs(1));
    assert!(
        !rules.chain_ahead(later + HINT_WINDOW),
        "hinted within the window"
    );
    let lost = later + HINT_WINDOW + HINT_WINDOW;
    rules.chain_changed("friend", lost);
    assert!(rules.chain_ahead(lost + HINT_WINDOW), "the hint never came");
    let mut young = InboundLiveness::new(start);
    young.chain_changed("friend", start);
    assert!(
        !young.chain_ahead(start + HINT_WINDOW),
        "the friend holds the old blob"
    );
}
