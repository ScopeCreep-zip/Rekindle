use super::*;

#[test]
fn dedup_detects_duplicates() {
    let mut cache = DedupCache::new(10);
    assert!(!cache.check_and_insert("c1", "s1", "msg1"));
    assert!(cache.check_and_insert("c1", "s1", "msg1"));
    assert!(!cache.check_and_insert("c1", "s1", "msg2"));
}

#[test]
fn dedup_evicts_oldest_at_capacity() {
    let mut cache = DedupCache::new(2);
    assert!(!cache.check_and_insert("c1", "s1", "a"));
    assert!(!cache.check_and_insert("c1", "s1", "b"));
    assert!(!cache.check_and_insert("c1", "s1", "c")); // evicts "a"
    assert!(!cache.check_and_insert("c1", "s1", "a")); // "a" was evicted, no longer a dup
}

#[test]
fn lamport_clock_increment_and_merge() {
    let mut clock = LamportClock::new(0);
    assert_eq!(clock.increment(), Ok(1));
    assert_eq!(clock.increment(), Ok(2));
    assert_eq!(clock.merge(10), 11);
    assert_eq!(clock.merge(5), 12); // max(11, 5) + 1
}

#[test]
fn lamport_clock_clamps_forged_future_timestamp() {
    let mut clock = LamportClock::new(2);
    assert_eq!(
        clock.merge(u64::MAX),
        2 + rekindle_gossip::lamport::MAX_LAMPORT_DRIFT + 1
    );
}

#[test]
fn fanout_degree_thresholds() {
    assert_eq!(fanout_degree(0), 0);
    assert_eq!(fanout_degree(1), 1);
    // N ≤ 20 is min(N, 6), not N: the architecture's epidemic
    // parameters cap fan-out at 6 and rely on the 5-hop TTL to
    // finish the job. This crate previously asserted 20 here,
    // pinning its own deviation.
    assert_eq!(fanout_degree(7), 6);
    assert_eq!(fanout_degree(20), 6);
    assert_eq!(fanout_degree(21), 6);
    assert_eq!(fanout_degree(60), 6);
    assert_eq!(fanout_degree(61), 8);
    assert_eq!(fanout_degree(1000), 8);
}

// ── Cross-track parity ───────────────────────────────────────
//
// Fan-out degree and TTL are *protocol* parameters, not local
// tuning: they set how far a message travels through a mesh whose
// members run both this crate (daemon) and rekindle-gossip
// (desktop). Architecture "Epidemic broadcast parameters" fixes
// them together — D = min(N,6) for N ≤ 20, 6 for 21–60, 8 for 61+,
// TTL 5 — and rekindle-gossip's own note explains the coupling:
// "the dedup cache + 5-hop TTL guarantee delivery without flooding
// even when most peers don't receive a direct copy". Tuning one
// without the other silently changes coverage, so these assert
// against the shared implementations rather than against literals.

#[test]
fn fanout_degree_matches_the_shared_gossip_primitive() {
    for online in [0usize, 1, 5, 6, 7, 19, 20, 21, 60, 61, 1000] {
        assert_eq!(
            fanout_degree(online),
            rekindle_gossip::mesh::fanout_degree(online),
            "fan-out degree diverged from rekindle-gossip at N={online}"
        );
    }
}

#[test]
fn initial_gossip_ttl_is_the_shared_default() {
    // There is only one TTL constant now. This crate used to carry a
    // second (`broadcast::gossip::DEFAULT_TTL`, briefly 3 against
    // codec's 5) and this test pinned them together; the whole
    // postcard gossip stack it belonged to is gone, so the
    // assertion is that the envelope every track signs starts at the
    // architecture's five hops.
    assert_eq!(
        rekindle_codec::envelope::DEFAULT_TTL,
        5,
        "the mesh is specified at a 5-hop TTL"
    );
}

#[test]
fn rate_limiter_allows_within_limit() {
    // 10 msg/s floor: the 11th in the same instant is dropped.
    let mut rl = RateLimiter::new();
    let t0 = Instant::now();
    for i in 0..10 {
        assert!(rl.check_at("s1", t0), "message {i} within floor");
    }
    assert!(!rl.check_at("s1", t0), "11th in same instant is over floor");
}

#[test]
fn rate_limiter_resets_after_window() {
    let mut rl = RateLimiter::new();
    let t0 = Instant::now();
    for _ in 0..10 {
        assert!(rl.check_at("s1", t0));
    }
    assert!(!rl.check_at("s1", t0));
    // Refill is continuous, not a window reset: a second later the
    // bucket is back to full capacity.
    assert!(rl.check_at("s1", t0 + Duration::from_secs(1)));
}

// ── Receiver-side admission ──────────────────────────────────

#[test]
fn admit_gossip_accepts_normal_traffic_and_advances_the_clock() {
    let mut mesh = GossipMesh::new("c1".to_string());
    let t0 = Instant::now();
    assert_eq!(
        mesh.admit_gossip_at("alice", 1, t0),
        GossipAdmission::Accept
    );
    // The clock must actually move — the whole point of merging is
    // that our next send is ordered after what we just received.
    assert_eq!(mesh.clock.current(), 2, "merge(1) on a 0 clock → 2");
    assert_eq!(
        mesh.admit_gossip_at("alice", 9, t0),
        GossipAdmission::Accept
    );
    assert_eq!(mesh.clock.current(), 10);
}

#[test]
fn admit_gossip_rate_limits_a_flooding_sender() {
    let mut mesh = GossipMesh::new("c1".to_string());
    let t0 = Instant::now();
    for i in 0..10 {
        assert_eq!(
            mesh.admit_gossip_at("flooder", i + 1, t0),
            GossipAdmission::Accept
        );
    }
    assert_eq!(
        mesh.admit_gossip_at("flooder", 11, t0),
        GossipAdmission::RateLimited
    );
    // A different sender still gets through — the floor is
    // per-sender, so one flooder cannot mute the community.
    assert_eq!(mesh.admit_gossip_at("bob", 11, t0), GossipAdmission::Accept);
}

#[test]
fn admit_gossip_admits_forged_future_lamport_but_bounds_the_clock() {
    let mut mesh = GossipMesh::new("c1".to_string());
    let t0 = Instant::now();
    assert_eq!(
        mesh.admit_gossip_at("alice", 5, t0),
        GossipAdmission::Accept
    );
    let before = mesh.clock.current();
    assert_eq!(
        mesh.admit_gossip_at("mallory", u64::MAX, t0),
        GossipAdmission::Accept
    );
    assert_eq!(
        mesh.clock.current(),
        before + rekindle_gossip::lamport::MAX_LAMPORT_DRIFT + 1,
        "one forged message moves the clock by at most the drift bound"
    );
}

#[test]
fn rate_limiter_meters_each_sender_separately() {
    // One flooding sender must not consume another's budget.
    let mut rl = RateLimiter::new();
    let t0 = Instant::now();
    for _ in 0..10 {
        assert!(rl.check_at("flooder", t0));
    }
    assert!(!rl.check_at("flooder", t0));
    assert!(rl.check_at("quiet", t0), "unrelated sender is unaffected");
}
