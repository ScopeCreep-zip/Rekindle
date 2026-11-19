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

/// A timer in words, as the frontends show it: "off", "30 seconds",
/// "1 week", "90 minutes" (its largest whole unit).
#[must_use]
pub fn duration_text(seconds: u32) -> String {
    if seconds == 0 {
        return "off".into();
    }
    let (unit, one, many, _) = UNITS
        .into_iter()
        .find(|(u, ..)| seconds.is_multiple_of(*u))
        .unwrap_or(UNITS[4]);
    let n = seconds / unit;
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The time left until `expire_at` at `now` (both ms), rounded up in its
/// largest unit: "4w", "3d", "5h", "12m", "30s".
#[must_use]
pub fn remaining_short(expire_at: i64, now: i64) -> String {
    let ms = expire_at.saturating_sub(now).max(0);
    let seconds = u32::try_from(ms.saturating_add(999) / 1000).unwrap_or(u32::MAX);
    let (unit, .., short) = UNITS
        .into_iter()
        .find(|(u, ..)| seconds >= *u)
        .unwrap_or(UNITS[4]);
    format!("{}{short}", seconds.div_ceil(unit))
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
        assert_eq!(duration_text(5_400), "90 minutes");
        assert_eq!(duration_text(MAX_SECONDS), "4 weeks");
        assert_eq!(remaining_short(61_000, 0), "2m");
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
}
