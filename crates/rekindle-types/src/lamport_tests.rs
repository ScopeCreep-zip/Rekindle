use super::{LamportClock, MAX_LAMPORT_DRIFT};

#[test]
fn merge_is_max_plus_one() {
    let mut clock = LamportClock::new(7);
    assert_eq!(clock.merge(3), 8);
    assert_eq!(clock.merge(11), 12);
    assert_eq!(clock.current(), 12);
}

#[test]
fn merge_clamps_drift_above_cap() {
    // M9.2 — a peer claiming a Lamport value far ahead of ours moves
    // the clock by at most MAX_LAMPORT_DRIFT + 1.
    let mut clock = LamportClock::new(100);
    assert_eq!(clock.merge(100 + 1_000_000), 100 + MAX_LAMPORT_DRIFT + 1);
}

#[test]
fn merge_at_cap_boundary() {
    let mut clock = LamportClock::new(100);
    let edge = 100 + MAX_LAMPORT_DRIFT;
    assert_eq!(clock.merge(edge), edge + 1);
}

#[test]
fn forged_u64_max_cannot_pin_the_clock() {
    let mut clock = LamportClock::new(5);
    assert_eq!(clock.merge(u64::MAX), 5 + MAX_LAMPORT_DRIFT + 1);
    // At the ceiling a merge that would pass u64::MAX leaves the clock
    // where it is instead of wrapping.
    let mut high = LamportClock::new(u64::MAX - 1);
    assert_eq!(high.merge(u64::MAX), u64::MAX - 1);
}

#[test]
fn increment_advances_by_one_and_stops_at_the_ceiling() {
    let mut clock = LamportClock::new(5);
    assert_eq!(clock.increment(), Ok(6));
    assert_eq!(clock.increment(), Ok(7));
    let mut top = LamportClock::new(u64::MAX);
    assert_eq!(top.increment(), Err(super::LamportError::Exhausted));
    assert_eq!(top.current(), u64::MAX);
}
