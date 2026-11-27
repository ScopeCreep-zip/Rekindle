use std::time::Duration;

use super::*;

#[test]
fn a_dead_blob_stays_dead_until_the_failure_cache_expires() {
    let t0 = Instant::now();
    let mut dead = DeadRoutes::default();
    assert!(dead.declare(b"A", t0));
    assert!(!dead.declare(b"A", t0), "already dead");
    assert!(dead.is_dead(
        b"A",
        t0 + FAILURE_CACHE.saturating_sub(Duration::from_secs(1))
    ));
    assert!(!dead.is_dead(b"B", t0));
    assert!(!dead.is_dead(b"A", t0 + FAILURE_CACHE), "expired");
    assert!(dead.declare(b"A", t0 + FAILURE_CACHE), "declared again");
}

#[test]
fn only_a_strictly_higher_seq_after_the_baseline_clears_a_dead_blob() {
    let t0 = Instant::now();
    let mut dead = DeadRoutes::default();
    dead.declare(b"A", t0);
    assert!(!dead.observe(b"A", 7), "the first read is the baseline");
    assert!(!dead.observe(b"A", 7), "the same record");
    assert!(!dead.observe(b"A", 6), "a lagging replica");
    assert!(dead.is_dead(b"A", t0));
    assert!(dead.observe(b"A", 8), "republished");
    assert!(!dead.is_dead(b"A", t0));
    assert!(!dead.observe(b"B", 9), "never dead");
}

#[test]
fn an_answered_call_proves_a_dead_blob() {
    let t0 = Instant::now();
    let mut dead = DeadRoutes::default();
    dead.declare(b"A", t0);
    assert!(dead.proven(b"A"));
    assert!(!dead.is_dead(b"A", t0));
    assert!(!dead.proven(b"A"), "no longer dead");
}
