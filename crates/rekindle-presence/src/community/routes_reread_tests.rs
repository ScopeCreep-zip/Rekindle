use std::time::Duration;

use tokio::time::Instant;

use super::{MemberRouteCache, RouteVersion};
use crate::community::{REREAD_BACKOFF_MAX, REREAD_BACKOFF_MIN};

#[test]
fn an_unproductive_reread_waits_out_a_doubling_window_up_to_the_max() {
    let mut cache = MemberRouteCache::default();
    let mut now = Instant::now();
    let mut window = REREAD_BACKOFF_MIN;
    for _ in 0..10 {
        cache.finish_reread("record", false, now);
        assert!(!cache.may_reread("record", now + window - Duration::from_secs(1)));
        assert!(cache.may_reread("record", now + window));
        now += window;
        window = (window * 2).min(REREAD_BACKOFF_MAX);
    }
    assert_eq!(window, REREAD_BACKOFF_MAX);
}

#[test]
fn d8_holds_by_record_and_only_an_advance_lifts_its_cap() {
    let mut cache = MemberRouteCache::default();
    let now = Instant::now();
    let old = RouteVersion {
        seq: 4,
        blob: vec![1],
    };
    let new = RouteVersion {
        seq: 5,
        blob: vec![2],
    };
    assert!(cache.verify("record", &old));
    cache.finish_reread("record", false, now);
    assert!(cache.verify("other", &new));
    assert!(cache.may_reread("other", now));
    assert!(cache.verify("record", &old));
    assert!(!cache.may_reread("record", now), "the same seq again");
    assert!(cache.verify("record", &new));
    assert!(cache.may_reread("record", now), "a newer value lifts it");
}
