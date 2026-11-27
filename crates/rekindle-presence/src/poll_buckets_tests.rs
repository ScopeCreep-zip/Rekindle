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
