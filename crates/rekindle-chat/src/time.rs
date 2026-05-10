//! Crate-internal timestamp utilities.
//!
//! Single source of truth for wall-clock timestamps used across all
//! chat modules. Millisecond precision from UNIX epoch.
//!
//! The `as u64` truncation is safe — milliseconds since epoch won't
//! exceed u64::MAX until year 584,942,417 CE.

/// Current wall-clock time in milliseconds since UNIX epoch.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Current wall-clock time in seconds since UNIX epoch, as i64.
///
/// Used by PQXDH prekey bundle `published_at` which requires i64
/// for cross-language compatibility with protobuf Timestamp.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
pub(crate) fn timestamp_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Current wall-clock time in milliseconds since UNIX epoch, as i64.
///
/// Used by presence payloads where the DHT subkey format uses
/// signed 8-byte big-endian timestamps.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn timestamp_ms_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
