use rekindle_transport_ipc::v3::handoff::credit::{SideChannelCreditTracker, CreditError};

#[test]
fn initial_credit_available() {
    let mut tracker = SideChannelCreditTracker::new(8);
    assert_eq!(tracker.remaining(), 8);
    for _ in 0..8 {
        assert!(tracker.try_consume().is_ok());
    }
    assert_eq!(tracker.remaining(), 0);
}

#[test]
fn credit_exhaustion() {
    let mut tracker = SideChannelCreditTracker::new(2);
    tracker.try_consume().unwrap();
    tracker.try_consume().unwrap();
    let err = tracker.try_consume().unwrap_err();
    assert!(matches!(err, CreditError::Exhausted));
}

#[test]
fn replenish_restores_credit() {
    let mut tracker = SideChannelCreditTracker::new(2);
    tracker.try_consume().unwrap();
    tracker.try_consume().unwrap();
    tracker.replenish(4, 1);
    for _ in 0..4 {
        assert!(tracker.try_consume().is_ok());
    }
    assert!(tracker.try_consume().is_err());
}

#[test]
fn stale_generation_ignored() {
    let mut tracker = SideChannelCreditTracker::new(2);
    tracker.try_consume().unwrap();
    tracker.try_consume().unwrap();
    tracker.replenish(10, 5);
    tracker.replenish(100, 3); // stale
    assert_eq!(tracker.remaining(), 10);
    assert_eq!(tracker.generation(), 5);
}

#[test]
fn generation_advances() {
    let mut tracker = SideChannelCreditTracker::new(1);
    assert_eq!(tracker.generation(), 0);
    tracker.replenish(5, 1);
    assert_eq!(tracker.generation(), 1);
    tracker.replenish(5, 2);
    assert_eq!(tracker.generation(), 2);
}

#[test]
fn zero_initial_credit() {
    let mut tracker = SideChannelCreditTracker::new(0);
    assert!(tracker.try_consume().is_err());
}

#[test]
fn replenish_replaces_not_adds() {
    let mut tracker = SideChannelCreditTracker::new(8);
    tracker.try_consume().unwrap(); // 7 remaining
    tracker.try_consume().unwrap(); // 6 remaining
    tracker.try_consume().unwrap(); // 5 remaining
    tracker.replenish(4, 1); // replaces: now 4, not 5+4=9
    assert_eq!(tracker.remaining(), 4);
}

#[test]
fn concurrent_exhaust_no_panic() {
    let mut tracker = SideChannelCreditTracker::new(3);
    for _ in 0..100 {
        let _ = tracker.try_consume();
    }
    assert_eq!(tracker.remaining(), 0);
}
