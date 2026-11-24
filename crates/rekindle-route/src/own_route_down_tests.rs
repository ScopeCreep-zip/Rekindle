use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::*;

fn downs() -> Downs {
    let wall = Arc::new(AtomicU64::new(1_000));
    Downs::new(Arc::new(move || wall.fetch_add(1, Ordering::SeqCst)))
}

const TEST_FAILED: &str = "allocated route failed to test";

/// Plan RV1g A1/A2: the down set changes once on the down transition,
/// never per attempt, and once on the way up.
#[tokio::test(start_paused = true)]
async fn a_down_class_is_told_once_each_way() {
    let downs = downs();
    let mut set = downs.subscribe();
    downs.lost(RouteClass::General, true);
    for attempt in 0..25 {
        downs.failed(RouteClass::General, &format!("{TEST_FAILED} #{attempt}"));
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(
            set.has_changed().unwrap(),
            attempt == 0,
            "attempt {attempt}"
        );
        set.borrow_and_update();
    }
    let outage = set.borrow().general.clone().expect("general is down");
    assert_eq!(outage.since_ms, 1_000);
    assert_eq!(outage.reason, format!("{TEST_FAILED} #0"));
    assert!(set.borrow().media.is_none());
    assert!(downs
        .up(RouteClass::General)
        .is_some_and(|d| d >= Duration::from_secs(120)));
    assert!(set.has_changed().unwrap());
    assert!(set.borrow_and_update().general.is_none());
    assert!(downs.up(RouteClass::General).is_none());
    assert!(!set.has_changed().unwrap());
}

/// Plan RV1g A4: after twenty failed tests in a row the retry steps to a
/// minute; another kind of failure breaks the streak.
#[test]
fn twenty_failed_tests_step_the_retry() {
    let downs = downs();
    for _ in 0..19 {
        assert!(!downs.failed(RouteClass::Media, TEST_FAILED).slow);
    }
    assert!(downs.failed(RouteClass::Media, TEST_FAILED).slow);
    assert!(downs.failed(RouteClass::Media, TEST_FAILED).slow);
    assert!(!downs.failed(RouteClass::Media, "too few nodes").slow);
    assert!(!downs.failed(RouteClass::Media, TEST_FAILED).slow);
}

/// Plan RV1g A3: only a Veilid-reported death of the General route
/// withdraws it, once, after the grace; a rotation and the media class
/// never do.
#[tokio::test(start_paused = true)]
async fn only_a_reported_general_death_withdraws_after_the_grace() {
    let downs = downs();
    downs.lost(RouteClass::General, true);
    let first = downs.failed(RouteClass::General, TEST_FAILED);
    assert!(!first.withdraw);
    assert_eq!(first.withdraw_in, Some(WITHDRAW_GRACE));
    tokio::time::advance(WITHDRAW_GRACE).await;
    assert!(downs.failed(RouteClass::General, TEST_FAILED).withdraw);
    let after = downs.failed(RouteClass::General, TEST_FAILED);
    assert!(
        !after.withdraw && after.withdraw_in.is_none(),
        "withdrawn twice"
    );

    let rotated = super::Downs::new(Arc::new(|| 0));
    rotated.lost(RouteClass::General, false);
    rotated.lost(RouteClass::Media, true);
    tokio::time::advance(WITHDRAW_GRACE * 3).await;
    for class in [RouteClass::General, RouteClass::Media] {
        let failure = rotated.failed(class, TEST_FAILED);
        assert!(
            !failure.withdraw && failure.withdraw_in.is_none(),
            "{class:?}"
        );
    }
}
