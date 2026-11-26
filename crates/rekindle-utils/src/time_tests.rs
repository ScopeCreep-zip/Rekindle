use super::{Clock, SystemClock};

#[test]
fn the_system_clock_agrees_with_the_free_functions() {
    let clock = SystemClock;
    let (before, now, after) = (super::timestamp_ms(), clock.now_ms(), super::timestamp_ms());
    assert!(before <= now && now <= after);
    assert!(clock.now_secs().abs_diff(super::timestamp_secs()) <= 1);
}
