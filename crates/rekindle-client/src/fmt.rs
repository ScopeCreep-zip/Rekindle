//! Display formatting shared by the frontends.

use std::time::Duration;

/// What a frontend shows in place of a post the D26 gate hid (an automod
/// `BlurContent` match, `automod_blurred`): the body stays out of sight.
pub const AUTOMOD_BLURRED: &str = "[blurred by automod]";

/// The mark after an edited channel message's body (plan E3.3, the GUI's
/// "(edited)").
pub const EDITED: &str = "(edited)";

/// What an author's own delete did (plan E3.3 E4): "deleted for members",
/// never "erased" (old ciphertext stays in the author's record history and
/// in peers' copies).
pub const DELETED_FOR_MEMBERS: &str = "deleted for members";

/// The line in place of a moderator-removed message (plan E3.3 M4), as the
/// GUI's bubble shows it.
#[must_use]
pub fn removed_by(moderator_name: &str) -> String {
    format!("Removed by {moderator_name}")
}

/// A member's name in a community for display: the name `names` holds for
/// `pseudonym`, else its abbreviated key (the GUI bubble's fallback).
#[must_use]
pub fn member_name<S: std::hash::BuildHasher>(
    names: &std::collections::HashMap<String, String, S>,
    pseudonym: &str,
) -> String {
    names.get(pseudonym).filter(|n| !n.is_empty()).map_or_else(
        || abbreviate_key(pseudonym),
        |n| rekindle_utils::text::sanitize_for_display(n),
    )
}

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

/// A send the Host keeps retrying, worded (plan SR2): the attempts so
/// far and the next try's local `HH:MM:SS` (retries back off 1-300 s).
#[must_use]
pub fn delivery_retry_text(retry: &rekindle_types::message::DeliveryRetry) -> String {
    use chrono::{Local, TimeZone};
    let at = match Local.timestamp_millis_opt(retry.next_attempt_at).single() {
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => "??:??:??".to_string(),
    };
    format!(
        "Not delivered yet \u{2014} still trying (attempt {}, next try at {at})",
        retry.attempts
    )
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
    fn delivery_retry() {
        use chrono::{Local, TimeZone};
        let at = Local.with_ymd_and_hms(2026, 11, 22, 14, 5, 9).unwrap();
        let retry = rekindle_types::message::DeliveryRetry {
            attempts: 3,
            next_attempt_at: at.timestamp_millis(),
        };
        assert_eq!(
            delivery_retry_text(&retry),
            "Not delivered yet \u{2014} still trying (attempt 3, next try at 14:05:09)"
        );
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
        let key = rekindle_test_fixtures::HEX_KEY_64;
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
