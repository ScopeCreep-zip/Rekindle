//! Auto-away threshold decisions.
//!
//! Pure: given how long the user has been idle + the configured
//! threshold + their current status, decide whether to flip to Away
//! (or back to Online when they return).

use crate::status::UserStatusKind;

/// Default auto-away threshold (10 minutes of input inactivity).
pub const IDLE_THRESHOLD_MS: u64 = 10 * 60 * 1000;

/// Decide the next status given idle-ms input + threshold + current
/// status. Pure — no side effects.
///
/// Rules:
/// - Idle ≥ threshold AND currently Online → Away
/// - Idle < threshold AND currently Away → Online (returning user)
/// - All other transitions return `None` (no change)
#[must_use]
pub fn decide_status_after_idle(
    current: UserStatusKind,
    idle_ms: u64,
    threshold_ms: u64,
) -> Option<UserStatusKind> {
    match current {
        UserStatusKind::Online if idle_ms >= threshold_ms => Some(UserStatusKind::Away),
        UserStatusKind::Away if idle_ms < threshold_ms => Some(UserStatusKind::Online),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
            decide_status_after_idle(UserStatusKind::Away, 12 * 60 * 1000, IDLE_THRESHOLD_MS)
                .is_none()
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
}
