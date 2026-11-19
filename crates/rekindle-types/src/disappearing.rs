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
}
