use std::time::{SystemTime, UNIX_EPOCH};

// This module is the canonical `SystemTime::now()` wrapper for the
// workspace. The workspace `clippy.toml` forbids direct
// `std::time::SystemTime::now` and redirects callers here; that
// redirect target must itself call `SystemTime::now` to do its job.
//
// Mirrors Veilid's same pattern: `veilid-tools::raw_timestamp.rs:66`
// is the one place in `veilid-tools` that calls
// `SystemTime::now().duration_since(UNIX_EPOCH)`; every other module
// in the Veilid workspace routes through `Timestamp::now()`. The
// `#[allow]`s below are scoped per-function, not module-wide, so a
// future contributor cannot accidentally smuggle a third call site
// past review.

/// Current UNIX timestamp in **milliseconds** as `u64`.
///
/// Used for protocol timestamps (Cap'n Proto messages, DHT records, voice packets).
#[allow(
    clippy::disallowed_methods,
    reason = "Canonical SystemTime wrapper — see module-level note"
)]
pub fn timestamp_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

/// Current UNIX timestamp in **milliseconds** as `i64`.
///
/// Used for `SQLite` columns that store millisecond-precision timestamps.
#[allow(
    clippy::disallowed_methods,
    reason = "Canonical SystemTime wrapper — see module-level note"
)]
pub fn timestamp_ms_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

/// Current UNIX timestamp in **seconds** as `u64`.
///
/// Used for timeouts, server uptime, and community timeout computation.
#[allow(
    clippy::disallowed_methods,
    reason = "Canonical SystemTime wrapper — see module-level note"
)]
pub fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Current UNIX timestamp in **seconds** as `i64`.
///
/// Used for `SQLite` columns that store second-precision timestamps (server crate).
#[allow(
    clippy::disallowed_methods,
    reason = "Canonical SystemTime wrapper — see module-level note"
)]
pub fn timestamp_secs_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}
