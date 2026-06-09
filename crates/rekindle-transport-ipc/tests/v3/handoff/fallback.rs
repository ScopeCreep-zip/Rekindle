use rekindle_transport_ipc::v3::handoff::fallback::{FallbackTracker, FallbackConfig};

#[test]
fn initially_enabled() {
    let tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 3 });
    assert!(!tracker.is_disabled());
    assert_eq!(tracker.consecutive_failures(), 0);
}

#[test]
fn success_resets_counter() {
    let mut tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 3 });
    tracker.record_failure();
    tracker.record_failure();
    assert_eq!(tracker.consecutive_failures(), 2);
    tracker.record_success();
    assert_eq!(tracker.consecutive_failures(), 0);
    assert!(!tracker.is_disabled());
}

#[test]
fn three_failures_disables() {
    let mut tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 3 });
    tracker.record_failure();
    tracker.record_failure();
    tracker.record_failure();
    assert!(tracker.is_disabled());
}

#[test]
fn two_failures_still_enabled() {
    let mut tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 3 });
    tracker.record_failure();
    tracker.record_failure();
    assert!(!tracker.is_disabled());
}

#[test]
fn reset_re_enables() {
    let mut tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 3 });
    tracker.record_failure();
    tracker.record_failure();
    tracker.record_failure();
    assert!(tracker.is_disabled());
    tracker.reset();
    assert!(!tracker.is_disabled());
    assert_eq!(tracker.consecutive_failures(), 0);
}

#[test]
fn consecutive_failures_tracked() {
    let mut tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 10 });
    tracker.record_failure();
    assert_eq!(tracker.consecutive_failures(), 1);
    tracker.record_failure();
    assert_eq!(tracker.consecutive_failures(), 2);
}

#[test]
fn success_after_disable_re_enables() {
    let mut tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 3 });
    tracker.record_failure();
    tracker.record_failure();
    tracker.record_failure();
    assert!(tracker.is_disabled());
    tracker.record_success();
    assert!(!tracker.is_disabled());
    assert_eq!(tracker.consecutive_failures(), 0);
}

#[test]
fn custom_threshold() {
    let mut tracker = FallbackTracker::new(FallbackConfig { max_consecutive_failures: 10 });
    for _ in 0..9 {
        tracker.record_failure();
    }
    assert!(!tracker.is_disabled());
    tracker.record_failure(); // 10th
    assert!(tracker.is_disabled());
}
