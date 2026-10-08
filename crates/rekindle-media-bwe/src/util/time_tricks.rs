//! Copied from str0m `src/util/time_tricks.rs` (`not_happening`, `already_happened`
//! and the `BEGINNING_OF_TIME` instant they need; the NTP/unix conversions are not
//! used by the estimator or the pacer and are not copied).

use std::sync::LazyLock;
use std::time::{Duration, Instant};

pub(crate) fn not_happening() -> Instant {
    const YEARS_100: Duration = Duration::from_secs(60 * 60 * 24 * 365 * 100);
    static FUTURE: LazyLock<Instant> = LazyLock::new(|| Instant::now() + YEARS_100);
    *FUTURE
}

// The goal here is to make a constant "beginning of time" in Instant that we can use
// as a relative value. str0m pairs it with a SystemTime for NTP conversions; only the
// Instant half is needed here.
static BEGINNING_OF_TIME: LazyLock<Instant> = LazyLock::new(|| {
    let now = Instant::now();

    // Find an Instant in the past which is up to an hour back.
    let mut secs = 3600;
    loop {
        let dur = Duration::from_secs(secs);
        if let Some(v) = now.checked_sub(dur) {
            break v;
        }
        secs -= 1;
        assert!(secs != 0, "Failed to find a beginning of time instant");
    }
});

pub(crate) fn already_happened() -> Instant {
    *BEGINNING_OF_TIME
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn not_happening_works() {
        assert_eq!(not_happening(), not_happening());
        assert!(Instant::now() < not_happening());
    }

    #[test]
    fn already_happened_works() {
        assert_eq!(already_happened(), already_happened());
        assert!(Instant::now() > already_happened());
    }

    #[test]
    fn already_happened_ne() {
        assert_ne!(not_happening(), already_happened());
    }
}
