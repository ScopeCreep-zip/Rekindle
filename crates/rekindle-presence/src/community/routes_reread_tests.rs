//! D8's cap on the member route re-read, and the dead-blob filter.

use std::time::Duration;

use rekindle_types::id::PseudonymKey;
use tokio::time::Instant;

use super::MemberRouteCache;
use crate::community::{REREAD_BACKOFF_MAX, REREAD_BACKOFF_MIN};

fn member(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

#[test]
fn a_reread_of_a_blob_the_transport_knows_dead_is_no_route() {
    let mut cache = MemberRouteCache::default();
    let now = Instant::now();
    let dead = |b: &[u8]| b == b"D";
    let read = Some((4, b"D".to_vec()));
    assert_eq!(cache.reread(&member(1), Some(b"A"), read, dead, now), None);
    assert!(
        !cache.may_reread(&member(1), now),
        "the dead blob capped the re-read"
    );
    let live = cache.reread(&member(2), Some(b"A"), Some((4, b"B".to_vec())), dead, now);
    assert_eq!(live, Some(b"B".to_vec()), "a live new blob is the route");
    assert!(
        cache.may_reread(&member(2), now),
        "a usable re-read caps nothing"
    );
}

#[test]
fn a_reread_of_the_failed_blob_or_nothing_is_no_route() {
    let mut cache = MemberRouteCache::default();
    let now = Instant::now();
    let read = Some((4, b"A".to_vec()));
    assert_eq!(
        cache.reread(&member(1), Some(b"A"), read, |_| false, now),
        None
    );
    assert_eq!(
        cache.reread(&member(2), Some(b"A"), None, |_| false, now),
        None
    );
    let empty = Some((4, Vec::new()));
    assert_eq!(cache.reread(&member(3), None, empty, |_| false, now), None);
}

#[test]
fn an_unproductive_reread_waits_out_a_doubling_window_up_to_the_max() {
    let mut cache = MemberRouteCache::default();
    let mut now = Instant::now();
    let mut window = REREAD_BACKOFF_MIN;
    for _ in 0..10 {
        let read = Some((4, b"A".to_vec()));
        assert_eq!(
            cache.reread(&member(1), Some(b"A"), read, |_| false, now),
            None
        );
        assert!(!cache.may_reread(&member(1), now + window - Duration::from_secs(1)));
        assert!(cache.may_reread(&member(1), now + window));
        now += window;
        window = (window * 2).min(REREAD_BACKOFF_MAX);
    }
    assert_eq!(window, REREAD_BACKOFF_MAX);
}

#[test]
fn a_newer_seq_observed_lifts_the_cap_and_an_equal_one_does_not() {
    let mut cache = MemberRouteCache::default();
    let now = Instant::now();
    let read = Some((4, b"A".to_vec()));
    cache.reread(&member(1), Some(b"A"), read, |_| false, now);
    cache.observe(&member(1), 4, b"A".to_vec());
    assert!(!cache.may_reread(&member(1), now), "the same seq again");
    cache.observe(&member(1), 5, b"B".to_vec());
    assert!(cache.may_reread(&member(1), now), "a newer value lifts it");
}
