//! Presence-poll bucket timing + stale-member TTL.
//!
//! Pure: per-community presence poll fires every 15s; presence rows
//! older than 60s are considered stale and get evicted from the
//! online-members map.

/// Architecture §13.4 — 15s community-presence poll cadence.
pub const PRESENCE_POLL_INTERVAL_MS: u64 = 15_000;

/// Architecture §13.4 — drop online-members entries whose `last_seen`
/// is older than 60s. Receivers re-add them on the next presence pong.
pub const STALE_MEMBER_TTL_MS: u64 = 60_000;

/// Returns the configured community-presence-poll interval in
/// milliseconds. Inline constant exported as a function for symmetry
/// with `is_member_stale` callers.
#[must_use]
pub const fn presence_poll_interval_ms() -> u64 {
    PRESENCE_POLL_INTERVAL_MS
}

/// Decide whether an online-members entry is stale and should be
/// evicted from the live map. Pure comparison — caller passes wall
/// clock; the helper handles saturating arithmetic for clock-skew
/// safety.
///
/// `last_seen_ms` is the timestamp from the registry/gossip pong;
/// `now_ms` is the current wall clock. Returns `true` when the
/// entry should be dropped.
#[must_use]
pub fn is_member_stale(last_seen_ms: u64, now_ms: u64, ttl_ms: u64) -> bool {
    now_ms.saturating_sub(last_seen_ms) > ttl_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_interval_is_fifteen_seconds() {
        assert_eq!(presence_poll_interval_ms(), 15_000);
        assert_eq!(PRESENCE_POLL_INTERVAL_MS, 15_000);
    }

    #[test]
    fn stale_ttl_is_one_minute() {
        assert_eq!(STALE_MEMBER_TTL_MS, 60_000);
    }

    #[test]
    fn fresh_member_not_stale() {
        assert!(!is_member_stale(1_000, 2_000, STALE_MEMBER_TTL_MS));
        assert!(!is_member_stale(0, 30_000, STALE_MEMBER_TTL_MS));
    }

    #[test]
    fn aged_member_is_stale() {
        assert!(is_member_stale(0, 60_001, STALE_MEMBER_TTL_MS));
        assert!(is_member_stale(1_000, 70_000, STALE_MEMBER_TTL_MS));
    }

    #[test]
    fn equal_ttl_is_not_stale_by_strict_greater_than() {
        // exactly TTL is the boundary — not yet stale
        assert!(!is_member_stale(0, 60_000, STALE_MEMBER_TTL_MS));
    }

    #[test]
    fn saturating_arithmetic_handles_clock_skew() {
        // last_seen "in the future" (clock skew) — elapsed = 0, never stale.
        assert!(!is_member_stale(100_000, 50_000, STALE_MEMBER_TTL_MS));
    }
}
