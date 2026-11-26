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

/// Plan RV2: two copies of one receipted call from a peer, over a route
/// older than one re-fetch gap, rotate it once; that peer cannot rotate it
/// again within its cooldown, across the new route; copies of different
/// frames and copies on a young route never count.
#[test]
fn re_sent_calls_rotate_once_per_peer_cooldown() {
    let start = Instant::now();
    let mut route = InboundLiveness::new(start);
    route.duplicate_call("ada", [1; 32], at(start, 30));
    route.duplicate_call("ada", [1; 32], at(start, 40));
    assert_eq!(route.receipt_lost(at(start, 61)), None, "a young route");
    route.duplicate_call("ada", [1; 32], at(start, 61));
    assert_eq!(route.receipt_lost(at(start, 61)), None, "one copy");
    route.duplicate_call("ada", [2; 32], at(start, 62));
    assert_eq!(route.receipt_lost(at(start, 62)), None, "another frame");
    route.duplicate_call("ada", [2; 32], at(start, 63));
    assert_eq!(route.receipt_lost(at(start, 63)).as_deref(), Some("ada"));
    route.rotated_for("ada", at(start, 63));
    route.route_replaced(at(start, 63));
    route.duplicate_call("ada", [3; 32], at(start, 200));
    route.duplicate_call("ada", [3; 32], at(start, 201));
    assert_eq!(route.receipt_lost(at(start, 201)), None, "ada is cooling");
    route.duplicate_call("bea", [3; 32], at(start, 202));
    route.duplicate_call("bea", [3; 32], at(start, 203));
    assert_eq!(
        route.receipt_lost(at(start, 203)).as_deref(),
        Some("bea"),
        "another peer's evidence still counts"
    );
    let cooled = at(start, 63) + REPORT_PEER_COOLDOWN;
    assert_eq!(route.receipt_lost(cooled).as_deref(), Some("ada"));
    assert_eq!(REPORT_PEER_COOLDOWN, Duration::from_secs(2 * 60 * 60));
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

/// Plan RV2, the sixth evidence source: a durable first contact whose call
/// copy never arrives over the route within the window rotates it, once
/// per peer cooldown; a call copy before or after it, or a route too young
/// for the peer to hold, does not.
#[test]
fn a_reply_without_its_call_rotates() {
    let start = Instant::now();
    let mut rules = InboundLiveness::new(start);
    let old = at(start, REPORT_ROTATE_AGE.as_secs());
    rules.call_seen([1; 32], old);
    rules.reply_landed("ann", [1; 32], old + Duration::from_secs(5));
    assert_eq!(
        rules.reply_ahead(old + HINT_WINDOW * 2),
        None,
        "called first"
    );
    rules.reply_landed("ann", [2; 32], old);
    rules.call_seen([2; 32], old + Duration::from_secs(29));
    assert_eq!(
        rules.reply_ahead(old + HINT_WINDOW * 2),
        None,
        "called after"
    );
    let lost = old + HINT_WINDOW * 3;
    rules.reply_landed("ann", [3; 32], lost);
    assert_eq!(rules.reply_ahead(lost), None, "the call may still come");
    assert_eq!(
        rules.reply_ahead(lost + HINT_WINDOW),
        Some("ann".to_owned())
    );
    rules.rotated_for("ann", lost + HINT_WINDOW);
    rules.reply_landed("ann", [4; 32], lost + HINT_WINDOW);
    assert_eq!(
        rules.reply_ahead(lost + HINT_WINDOW * 3),
        None,
        "ann is cooling"
    );
    let mut young = InboundLiveness::new(start);
    young.reply_landed("ann", [5; 32], start);
    assert_eq!(
        young.reply_ahead(start + HINT_WINDOW),
        None,
        "ann may hold the old blob"
    );
}

/// Plan RV2, the seventh evidence source: a peer's report rotates the route
/// only when it names our live blob, on a route old enough that the peer
/// holds it, once per peer cooldown; a report on an old blob, from a peer
/// inside its cooldown, or on a replaced route does nothing.
#[test]
fn a_report_on_our_live_blob_rotates_once_per_peer_cooldown() {
    let start = Instant::now();
    let (live, old_blob) = ([7; 32], [6; 32]);
    let mut rules = InboundLiveness::new(start);
    rules.route_reported("bea", old_blob, live, start);
    assert_eq!(
        rules.report_ahead(at(start, REPORT_ROTATE_AGE.as_secs())),
        None,
        "a report naming a blob we already replaced"
    );
    rules.route_reported("bea", live, live, start);
    assert_eq!(
        rules.report_ahead(start),
        None,
        "the route is younger than REPORT_ROTATE_AGE"
    );
    let old = at(start, REPORT_ROTATE_AGE.as_secs());
    assert_eq!(rules.report_ahead(old), Some("bea".to_owned()));
    rules.rotated_for("bea", old);
    assert_eq!(rules.report_ahead(old), None, "the report was spent");
    rules.route_reported("bea", live, live, old);
    assert_eq!(
        rules.report_ahead(old + REPORT_PEER_COOLDOWN - Duration::from_secs(1)),
        None,
        "bea is cooling"
    );
    assert_eq!(
        rules.report_ahead(old + REPORT_PEER_COOLDOWN),
        Some("bea".to_owned()),
        "bea's cooldown passed"
    );
    rules.route_replaced(old + REPORT_PEER_COOLDOWN);
    assert_eq!(
        rules.report_ahead(old + REPORT_PEER_COOLDOWN + REPORT_ROTATE_AGE),
        None,
        "a report on the replaced route is not about the new one"
    );
}
