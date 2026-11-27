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
#[path = "idle_tests.rs"]
mod tests;
