//! Disappearing-message timers (plan E7.1): the values a conversation
//! timer may take. Signal's presets (Signal-Android
//! `ExpireTimerSettingsFragment`: 30 s … 4 weeks) plus a custom value up
//! to four weeks, the plan's bound; 0 is off.

/// The longest timer: four weeks, in seconds.
pub const MAX_SECONDS: u32 = 4 * 7 * 24 * 60 * 60;

/// The presets the timer control offers, in seconds (0 = off).
pub const PRESETS: &[u32] = &[
    0,
    30,
    5 * 60,
    60 * 60,
    8 * 60 * 60,
    24 * 60 * 60,
    7 * 24 * 60 * 60,
    MAX_SECONDS,
];

/// Whether `seconds` is a timer a conversation may carry: off, or a
/// custom value of at most four weeks.
#[must_use]
pub fn valid(seconds: u32) -> bool {
    seconds <= MAX_SECONDS
}

/// When a message with `seconds` on its timer, started at `start_ms`, is
/// deleted (ms); `None` with no timer.
#[must_use]
pub fn expire_at(start_ms: i64, seconds: u32) -> Option<i64> {
    (seconds > 0).then(|| start_ms.saturating_add(i64::from(seconds) * 1000))
}

/// Units a timer is worded in, longest first: seconds, singular, plural,
/// short suffix.
const UNITS: [(u32, &str, &str, &str); 5] = [
    (7 * 24 * 60 * 60, "week", "weeks", "w"),
    (24 * 60 * 60, "day", "days", "d"),
    (60 * 60, "hour", "hours", "h"),
    (60, "minute", "minutes", "m"),
    (1, "second", "seconds", "s"),
];

/// `seconds` as the two largest units, the smaller rounded up: the index
/// of the larger unit in [`UNITS`], its count, and the count of the next
/// unit down (0 when there is none). 30 h is 1 day 6 hours; 29 h 59 m 1 s
/// rounds up to the same, and 6 d 23 h 30 m to 1 week. One rule for a timer
/// and for what is left of one, so a message's countdown starts on the
/// words its conversation's timer button shows.
fn parts(seconds: u32) -> (usize, u32, u32) {
    let top = UNITS
        .iter()
        .position(|(u, ..)| seconds >= *u)
        .unwrap_or(UNITS.len() - 1);
    let Some(&(small, ..)) = UNITS.get(top + 1) else {
        return (top, seconds, 0);
    };
    let rounded = seconds.div_ceil(small).saturating_mul(small);
    if top > 0 && rounded >= UNITS[top - 1].0 {
        // Rounding up carried into the larger unit (exactly one of it).
        return parts(rounded);
    }
    let unit = UNITS[top].0;
    (top, rounded / unit, (rounded % unit) / small)
}

/// `count` of the unit at `index` in words: "1 day", "6 hours".
fn words(index: usize, count: u32) -> String {
    let (_, one, many, _) = UNITS[index];
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `seconds` in its two largest units, in words ([`parts`]): "30 seconds",
/// "1 week", "1 hour 30 minutes", "1 day 6 hours".
fn in_words(seconds: u32) -> String {
    let (top, hi, lo) = parts(seconds);
    if lo == 0 {
        words(top, hi)
    } else {
        format!("{} {}", words(top, hi), words(top + 1, lo))
    }
}

/// A timer in words, as the frontends show it: "off", "30 seconds",
/// "1 week", "1 hour 30 minutes", "1 day 6 hours" (its two largest units,
/// the same rule as the countdown, [`remaining_text`]).
#[must_use]
pub fn duration_text(seconds: u32) -> String {
    if seconds == 0 {
        return "off".into();
    }
    in_words(seconds)
}

/// The whole seconds left until `expire_at` at `now` (both ms), rounded up.
fn seconds_left(expire_at: i64, now: i64) -> u32 {
    let ms = expire_at.saturating_sub(now).max(0);
    u32::try_from(ms.saturating_add(999) / 1000).unwrap_or(u32::MAX)
}

/// The time left until `expire_at` at `now` (both ms), in its two largest
/// units, the smaller rounded up ([`parts`]): "4w", "1d 6h", "12m", "30s".
#[must_use]
pub fn remaining_short(expire_at: i64, now: i64) -> String {
    let (top, hi, lo) = parts(seconds_left(expire_at, now));
    let short = |index: usize, count: u32| format!("{count}{}", UNITS[index].3);
    if lo == 0 {
        short(top, hi)
    } else {
        format!("{} {}", short(top, hi), short(top + 1, lo))
    }
}

/// The same in words, for a screen reader: "1 day 6 hours".
#[must_use]
pub fn remaining_text(expire_at: i64, now: i64) -> String {
    in_words(seconds_left(expire_at, now))
}

/// A timer change's timeline notice: `who` ("you" or a member's label) set
/// the timer to `seconds`.
#[must_use]
pub fn notice_text(who: &str, seconds: u32) -> String {
    if seconds == 0 {
        format!("{who} turned off disappearing messages")
    } else {
        format!(
            "{who} set disappearing messages to {}",
            duration_text(seconds)
        )
    }
}

#[cfg(test)]
mod tests {
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
}
