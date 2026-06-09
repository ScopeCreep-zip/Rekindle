use rekindle_transport_ipc::v3::stream::flow_control::CreditTracker;

#[test]
fn initial_credit_allows_first_chunks() {
    let mut t = CreditTracker::new(64, 64); // 64 chunk initial credit
    for _ in 0..64 {
        assert!(t.try_consume_chunk().is_ok());
    }
}

#[test]
fn chunk_beyond_initial_credit_blocked() {
    let mut t = CreditTracker::new(64, 64);
    for _ in 0..64 {
        t.try_consume_chunk().unwrap();
    }
    assert!(t.try_consume_chunk().is_err(), "65th chunk must be blocked");
}

#[test]
fn credit_replenishment_unblocks() {
    let mut t = CreditTracker::new(2, 2);
    t.try_consume_chunk().unwrap();
    t.try_consume_chunk().unwrap();
    assert!(t.try_consume_chunk().is_err());

    t.replenish(2, 1); // 2 more chunks at generation 1
    assert!(t.try_consume_chunk().is_ok());
    assert!(t.try_consume_chunk().is_ok());
    assert!(t.try_consume_chunk().is_err());
}

#[test]
fn lane_credit_bounds_aggregate() {
    // Lane credit 4, stream credit 10 — effective is 4
    let mut t = CreditTracker::with_lane_limit(10, 10, 4);
    for _ in 0..4 {
        assert!(t.try_consume_chunk().is_ok());
    }
    assert!(t.try_consume_chunk().is_err(), "lane credit exhausted");
}

#[test]
fn stream_credit_and_lane_credit_min() {
    // Stream credit 3, lane credit 100 — effective is 3
    let mut t = CreditTracker::with_lane_limit(3, 3, 100);
    for _ in 0..3 {
        assert!(t.try_consume_chunk().is_ok());
    }
    assert!(t.try_consume_chunk().is_err(), "stream credit exhausted");
}

#[test]
fn stale_credit_rejected() {
    let mut t = CreditTracker::new(2, 2);
    t.try_consume_chunk().unwrap();
    t.try_consume_chunk().unwrap();

    // Replenish at generation 5
    t.replenish(10, 5);

    // Try to replenish at older generation 3 — must be ignored
    t.replenish(100, 3);

    // Should have only 10 credits from gen 5, not 100 from gen 3
    for _ in 0..10 {
        assert!(t.try_consume_chunk().is_ok());
    }
    assert!(t.try_consume_chunk().is_err());
}

#[test]
fn credit_generation_monotonic() {
    let mut t = CreditTracker::new(0, 0);
    t.replenish(5, 1);
    t.replenish(5, 2);
    t.replenish(5, 1); // older generation — ignored
    // Total from gen 1 + gen 2 = 10, gen 1 replay ignored
    // Actually each replenish sets remaining, doesn't add
    // The tracker should report the latest generation's credit
    assert_eq!(t.current_generation(), 2);
}

#[test]
fn remaining_credit_reported() {
    let mut t = CreditTracker::new(10, 10);
    assert_eq!(t.remaining_chunks(), 10);
    t.try_consume_chunk().unwrap();
    assert_eq!(t.remaining_chunks(), 9);
}
