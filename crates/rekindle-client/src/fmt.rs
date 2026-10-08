//! Display formatting shared by the frontends.

use std::time::Duration;

/// A duration as "just now", "4m ago", "2h 13m ago", "3d 5h ago".
#[must_use]
pub fn format_duration_ago(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    if hours < 24 {
        if rem_mins > 0 {
            return format!("{hours}h {rem_mins}m ago");
        }
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    let rem_hours = hours % 24;
    if rem_hours > 0 {
        format!("{days}d {rem_hours}h ago")
    } else {
        format!("{days}d ago")
    }
}

/// An epoch timestamp (milliseconds) as local `YYYY-MM-DD HH:MM:SS`.
#[must_use]
pub fn format_timestamp(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(epoch_ms.cast_signed()).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{epoch_ms}ms"),
    }
}

/// An epoch timestamp (milliseconds) as local `HH:MM`.
#[must_use]
pub fn format_time_short(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(epoch_ms.cast_signed()).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => "??:??".to_string(),
    }
}

/// Bytes as "42 B", "1.2 KB", "3.4 MB", "1.1 GB".
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes < KB {
        return format!("{bytes} B");
    }
    // One decimal place by fixed-point integer math, so no
    // precision-losing float cast: tenths = value * 10 / divisor.
    let (divisor, unit) = if bytes < MB {
        (KB, "KB")
    } else if bytes < GB {
        (MB, "MB")
    } else {
        (GB, "GB")
    };
    let whole = bytes / divisor;
    let tenths = (bytes % divisor) * 10 / divisor;
    format!("{whole}.{tenths} {unit}")
}

/// A key shortened for display: first 8 characters, "…", last 4.
#[must_use]
pub fn abbreviate_key(key: &str) -> String {
    rekindle_utils::text::abbreviate(key, 8, 4)
}

/// An uptime as "42s", "12m 34s", "5h 12m", "3d 5h".
#[must_use]
pub fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        let rem = secs % 60;
        return format!("{mins}m {rem}s");
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    if hours < 24 {
        return format!("{hours}h {rem_mins}m");
    }
    let days = hours / 24;
    let rem_hours = hours % 24;
    format!("{days}d {rem_hours}h")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(format_duration_ago(Duration::from_secs(30)), "just now");
        assert_eq!(format_duration_ago(Duration::from_secs(300)), "5m ago");
        assert_eq!(format_duration_ago(Duration::from_secs(7380)), "2h 3m ago");
        assert_eq!(format_duration_ago(Duration::from_secs(90000)), "1d 1h ago");
    }

    #[test]
    fn bytes() {
        assert_eq!(format_bytes(42), "42 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }

    #[test]
    fn keys() {
        assert_eq!(abbreviate_key("abcdef"), "abcdef");
        let key = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        assert_eq!(abbreviate_key(key), "abcdef12…7890");
    }

    #[test]
    fn uptimes() {
        assert_eq!(format_uptime(42), "42s");
        assert_eq!(format_uptime(754), "12m 34s");
        assert_eq!(format_uptime(18720), "5h 12m");
        assert_eq!(format_uptime(277_200), "3d 5h");
    }
}
