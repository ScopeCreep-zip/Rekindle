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
    assert_eq!(
        dead.observe(b"A", 7, t0),
        DeadRouteVerdict::Held,
        "the first read is the baseline"
    );
    assert_eq!(
        dead.observe(b"A", 7, t0),
        DeadRouteVerdict::Held,
        "the same record"
    );
    assert_eq!(
        dead.observe(b"A", 6, t0),
        DeadRouteVerdict::Held,
        "a lagging replica"
    );
    assert!(dead.is_dead(b"A", t0));
    assert_eq!(
        dead.observe(b"A", 8, t0),
        DeadRouteVerdict::Cleared,
        "republished"
    );
    assert!(!dead.is_dead(b"A", t0));
    assert_eq!(
        dead.observe(b"B", 9, t0),
        DeadRouteVerdict::NotDead,
        "never dead"
    );
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

/// Plan RH1r-c S2/S3 for a peer that never republishes (inventory item
/// 25): its blob read again at the same seq one `REFETCH_GAP` after the
/// death is probed once, once per gap, never before the gap, never at a
/// lagging seq, and not past the failure cache.
#[test]
fn the_same_seq_one_refetch_gap_after_the_death_asks_for_one_probe() {
    let t0 = Instant::now();
    let second = Duration::from_secs(1);
    let mut dead = DeadRoutes::default();
    dead.declare(b"A", t0);
    assert_eq!(
        dead.observe(b"A", 7, t0),
        DeadRouteVerdict::Held,
        "the baseline"
    );
    let early = t0 + REFETCH_GAP.saturating_sub(second);
    assert_eq!(
        dead.observe(b"A", 7, early),
        DeadRouteVerdict::Held,
        "inside the gap"
    );
    let gap = t0 + REFETCH_GAP;
    assert_eq!(
        dead.observe(b"A", 6, gap),
        DeadRouteVerdict::Held,
        "a lagging replica"
    );
    assert_eq!(
        dead.observe(b"A", 7, gap),
        DeadRouteVerdict::Probe,
        "one gap after the death"
    );
    assert_eq!(
        dead.observe(b"A", 7, gap),
        DeadRouteVerdict::Held,
        "one probe per gap"
    );
    assert!(dead.is_dead(b"A", gap), "a probe does not clear it");
    let next = gap + REFETCH_GAP.saturating_sub(second);
    assert_eq!(
        dead.observe(b"A", 7, next),
        DeadRouteVerdict::Held,
        "the next gap not up"
    );
    assert_eq!(
        dead.observe(b"A", 7, t0 + FAILURE_CACHE),
        DeadRouteVerdict::NotDead,
        "expired"
    );
}
