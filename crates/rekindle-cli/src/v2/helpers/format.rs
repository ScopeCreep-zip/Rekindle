//! Human-readable formatting for durations, timestamps, bytes, keys, uptime.

use std::path::Path;
use std::time::Duration;

/// "just now", "4m ago", "2h 13m ago", "3d 5h ago"
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
        return if rem_mins > 0 {
            format!("{hours}h {rem_mins}m ago")
        } else {
            format!("{hours}h ago")
        };
    }
    let days = hours / 24;
    let rem_hours = hours % 24;
    if rem_hours > 0 { format!("{days}d {rem_hours}h ago") } else { format!("{days}d ago") }
}

/// "2026-05-09 14:31:00"
pub fn format_timestamp(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    #[allow(clippy::cast_possible_wrap)]
    match Local.timestamp_millis_opt(epoch_ms as i64).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{epoch_ms}ms"),
    }
}

/// "14:31"
pub fn format_time_short(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    #[allow(clippy::cast_possible_wrap)]
    match Local.timestamp_millis_opt(epoch_ms as i64).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => "??:??".to_string(),
    }
}

/// "42 B", "1.2 KB", "3.4 MB", "1.1 GB"
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 { return format!("{bytes} B"); }
    #[allow(clippy::cast_precision_loss)]
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 { return format!("{kb:.1} KB"); }
    let mb = kb / 1024.0;
    if mb < 1024.0 { return format!("{mb:.1} MB"); }
    format!("{:.1} GB", mb / 1024.0)
}

/// "abcdef12...7890" for strings longer than 16 chars.
/// Uses char_indices for safe truncation on non-ASCII input.
pub fn abbreviate_key(key: &str) -> String {
    if key.chars().count() > 16 {
        let start_end = key.char_indices().nth(8).map_or(key.len(), |(i, _)| i);
        let tail_start = key.char_indices().rev().nth(3).map_or(0, |(i, _)| i);
        format!("{}...{}", &key[..start_end], &key[tail_start..])
    } else {
        key.to_string()
    }
}

/// "42s", "12m 34s", "5h 12m", "3d 5h"
pub fn format_uptime(secs: u64) -> String {
    if secs < 60 { return format!("{secs}s"); }
    let mins = secs / 60;
    if mins < 60 { return format!("{mins}m {}s", secs % 60); }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    if hours < 24 { return format!("{hours}h {rem_mins}m"); }
    let days = hours / 24;
    format!("{days}d {}h", hours % 24)
}

/// Recursively compute directory size in bytes.
/// Uses symlink_metadata to avoid following symlinks (prevents infinite loops).
/// Depth-limited to 32 levels to bound stack usage.
pub fn dir_size(path: &Path) -> u64 {
    fn walk(path: &Path, depth: u32) -> u64 {
        if depth > 32 { return 0; }
        let Ok(entries) = std::fs::read_dir(path) else { return 0 };
        entries.filter_map(std::result::Result::ok).map(|e| {
            let Ok(meta) = e.path().symlink_metadata() else { return 0 };
            if meta.file_type().is_symlink() {
                0
            } else if meta.is_file() {
                meta.len()
            } else if meta.is_dir() {
                walk(&e.path(), depth + 1)
            } else {
                0
            }
        }).sum()
    }
    walk(path, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_ranges() {
        assert_eq!(format_duration_ago(Duration::from_secs(30)), "just now");
        assert_eq!(format_duration_ago(Duration::from_secs(300)), "5m ago");
        assert_eq!(format_duration_ago(Duration::from_secs(7380)), "2h 3m ago");
        assert_eq!(format_duration_ago(Duration::from_secs(90000)), "1d 1h ago");
    }

    #[test]
    fn bytes_units() {
        assert_eq!(format_bytes(42), "42 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }

    #[test]
    fn abbreviate_short_passthrough() {
        assert_eq!(abbreviate_key("abcdef"), "abcdef");
    }

    #[test]
    fn abbreviate_long_truncates() {
        let key = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        assert_eq!(abbreviate_key(key), "abcdef12...7890");
    }

    #[test]
    fn uptime_units() {
        assert_eq!(format_uptime(42), "42s");
        assert_eq!(format_uptime(754), "12m 34s");
        assert_eq!(format_uptime(18720), "5h 12m");
        assert_eq!(format_uptime(277200), "3d 5h");
    }
}
