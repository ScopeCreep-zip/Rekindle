use super::*;

#[test]
fn online_to_away_when_threshold_exceeded() {
    assert_eq!(
        decide_status_after_idle(UserStatusKind::Online, 11 * 60 * 1000, IDLE_THRESHOLD_MS),
        Some(UserStatusKind::Away)
    );
}

#[test]
fn online_stays_online_when_under_threshold() {
    assert!(
        decide_status_after_idle(UserStatusKind::Online, 5 * 60 * 1000, IDLE_THRESHOLD_MS)
            .is_none()
    );
}

#[test]
fn away_returns_to_online_when_active_again() {
    assert_eq!(
        decide_status_after_idle(UserStatusKind::Away, 0, IDLE_THRESHOLD_MS),
        Some(UserStatusKind::Online)
    );
}

#[test]
fn away_stays_away_while_still_idle() {
    assert!(
        decide_status_after_idle(UserStatusKind::Away, 12 * 60 * 1000, IDLE_THRESHOLD_MS).is_none()
    );
}

#[test]
fn busy_never_auto_flips() {
    assert!(decide_status_after_idle(UserStatusKind::Busy, 99 * 60 * 1000, 1).is_none());
    assert!(decide_status_after_idle(UserStatusKind::Busy, 0, 1).is_none());
}

#[test]
fn offline_never_auto_flips() {
    assert!(decide_status_after_idle(UserStatusKind::Offline, 99 * 60 * 1000, 1).is_none());
}

#[test]
fn invisible_never_auto_flips() {
    assert!(decide_status_after_idle(UserStatusKind::Invisible, 99 * 60 * 1000, 1).is_none());
}

#[test]
fn threshold_constant_is_ten_minutes() {
    assert_eq!(IDLE_THRESHOLD_MS, 600_000);
}
