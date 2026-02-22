use std::time::{SystemTime, UNIX_EPOCH};

/// Current UNIX timestamp in **milliseconds** as `u64`.
///
/// Used for protocol timestamps (Cap'n Proto messages, DHT records, voice packets).
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
pub fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Current UNIX timestamp in **seconds** as `i64`.
///
/// Used for `SQLite` columns that store second-precision timestamps (server crate).
pub fn timestamp_secs_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}
