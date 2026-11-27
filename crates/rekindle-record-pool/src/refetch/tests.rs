use super::{Unconfirmed, REFETCH_BACKOFF_MAX, REFETCH_BACKOFF_MIN};
use tokio::time::Instant;

#[test]
fn the_window_doubles_to_its_cap() {
    let (mut u, mut now) = (Unconfirmed::default(), Instant::now());
    let mut claim = 9;
    u.fetched("k", 0, claim, None, now);
    let mut window = REFETCH_BACKOFF_MIN;
    for _ in 0..10 {
        claim += 1;
        assert!(!u.may_fetch("k", 0, claim, now + window - REFETCH_BACKOFF_MIN / 60));
        now += window;
        assert!(u.may_fetch("k", 0, claim, now));
        u.fetched("k", 0, claim, Some(0), now);
        window = (window * 2).min(REFETCH_BACKOFF_MAX);
    }
    assert_eq!(window, REFETCH_BACKOFF_MAX);
    assert!(
        !u.may_fetch("k", 0, claim, now + REFETCH_BACKOFF_MAX * 10),
        "never the same claim"
    );
}

#[test]
fn a_close_forgets_its_records_claims_only() {
    let (mut u, now) = (Unconfirmed::default(), Instant::now());
    u.fetched("a", 0, 5, None, now);
    u.fetched("b", 0, 5, None, now);
    u.forget("a");
    assert!(u.may_fetch("a", 0, 5, now));
    assert!(!u.may_fetch("b", 0, 5, now));
}
