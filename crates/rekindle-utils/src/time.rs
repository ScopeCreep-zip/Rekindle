use std::time::{SystemTime, UNIX_EPOCH};

// This module is the canonical `SystemTime::now()` wrapper for the
// workspace. The workspace `clippy.toml` forbids direct
// `std::time::SystemTime::now` and redirects callers here; that
// redirect target must itself call `SystemTime::now` to do its job.
//
// Mirrors Veilid's same pattern: `veilid-tools::raw_timestamp.rs:66`
// is the one place in `veilid-tools` that calls
// `SystemTime::now().duration_since(UNIX_EPOCH)`; every other module
// in the Veilid workspace routes through `Timestamp::now()`. We follow
// that single-call-site shape: `since_epoch` below is the *only*
// `SystemTime::now` call in the workspace, so the unavoidable
// `disallowed_methods` exemption is confined to one private function
// instead of being repeated on every public accessor.

/// The sole `SystemTime::now()` call site in the workspace — every
/// public accessor derives from this. Returns the duration since the
/// UNIX epoch, saturating to zero if the system clock predates it.
#[allow(
    clippy::disallowed_methods,
    reason = "Canonical SystemTime wrapper — the one legal SystemTime::now call site"
)]
fn since_epoch() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Current UNIX timestamp in **milliseconds** as `u64`.
///
/// Used for protocol timestamps (Cap'n Proto messages, DHT records, voice packets).
pub fn timestamp_ms() -> u64 {
    u64::try_from(since_epoch().as_millis()).unwrap_or(u64::MAX)
}

/// Current UNIX timestamp in **milliseconds** as `i64`.
///
/// Used for `SQLite` columns that store millisecond-precision timestamps.
pub fn timestamp_ms_i64() -> i64 {
    i64::try_from(since_epoch().as_millis()).unwrap_or(i64::MAX)
}

/// Current UNIX timestamp in **seconds** as `u64`.
///
/// Used for timeouts, server uptime, and community timeout computation.
pub fn timestamp_secs() -> u64 {
    since_epoch().as_secs()
}

/// Current UNIX timestamp in **seconds** as `i64`.
///
/// Used for `SQLite` columns that store second-precision timestamps (server crate).
pub fn timestamp_secs_i64() -> i64 {
    i64::try_from(since_epoch().as_secs()).unwrap_or(i64::MAX)
}

/// Wall-clock time as the engines and the Host read it (plan C9.2): the
/// production [`SystemClock`], or a simulated clock, so a seeded run fixes
/// every timestamp the code writes or compares.
pub trait Clock: Send + Sync {
    /// Milliseconds since the UNIX epoch.
    fn now_ms(&self) -> u64;

    /// Seconds since the UNIX epoch.
    fn now_secs(&self) -> u64 {
        self.now_ms() / 1000
    }

    /// [`now_ms`](Self::now_ms) as `i64`, for `SQLite` columns.
    fn now_ms_i64(&self) -> i64 {
        i64::try_from(self.now_ms()).unwrap_or(i64::MAX)
    }
}

/// The system's wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        timestamp_ms()
    }

    fn now_secs(&self) -> u64 {
        timestamp_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, SystemClock};

    #[test]
    fn the_system_clock_agrees_with_the_free_functions() {
        let clock = SystemClock;
        let (before, now, after) = (super::timestamp_ms(), clock.now_ms(), super::timestamp_ms());
        assert!(before <= now && now <= after);
        assert!(clock.now_secs().abs_diff(super::timestamp_secs()) <= 1);
    }
}
