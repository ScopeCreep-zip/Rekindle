use super::*;

#[test]
fn four_weeks_is_the_bound_and_zero_is_off() {
    assert!(valid(0) && valid(MAX_SECONDS) && !valid(MAX_SECONDS + 1));
    assert!(PRESETS.iter().all(|s| valid(*s)));
    assert_eq!(expire_at(1_000, 0), None);
    assert_eq!(expire_at(1_000, 30), Some(31_000));
}

#[test]
fn timers_are_worded_in_their_largest_whole_unit() {
    assert_eq!(duration_text(0), "off");
    assert_eq!(duration_text(30), "30 seconds");
    assert_eq!(duration_text(5_400), "1 hour 30 minutes");
    assert_eq!(duration_text(MAX_SECONDS), "4 weeks");
    assert_eq!(remaining_short(61_000, 0), "1m 1s");
    assert_eq!(remaining_short(30_000, 0), "30s");
    assert_eq!(remaining_short(0, 5), "0s");
    assert_eq!(
        notice_text("you", 0),
        "you turned off disappearing messages"
    );
    assert_eq!(
        notice_text("bob", 3_600),
        "bob set disappearing messages to 1 hour"
    );
}

/// A 30-hour timer reads the same on the button and on a fresh
/// message's countdown (the Linux finding: "30 hours" on the button,
/// "2 days" on the bubble), and the countdown rounds up into the next
/// unit without ever showing more than the timer.
#[test]
fn the_countdown_starts_on_the_timer_words() {
    const HOUR: u32 = 60 * 60;
    for timer in PRESETS
        .iter()
        .copied()
        .filter(|s| *s > 0)
        .chain([30 * HOUR, 5_400, 100_001])
    {
        let at = expire_at(0, timer).expect("a timer");
        assert_eq!(remaining_text(at, 0), duration_text(timer), "{timer}");
    }
    assert_eq!(duration_text(30 * HOUR), "1 day 6 hours");
    assert_eq!(remaining_short(i64::from(30 * HOUR) * 1000, 0), "1d 6h");
    assert_eq!(remaining_short(i64::from(30 * HOUR) * 1000, 1), "1d 6h");
    assert_eq!(
        remaining_text(i64::from(7 * 24 * HOUR) * 1000, 30 * 60 * 1000),
        "1 week"
    );
    assert_eq!(
        remaining_text(i64::from(7 * 24 * HOUR) * 1000, i64::from(2 * HOUR) * 1000),
        "6 days 22 hours"
    );
    assert_eq!(remaining_text(1_000, 0), "1 second");
    assert_eq!(remaining_short(i64::from(HOUR) * 1000, 0), "1h");
}
