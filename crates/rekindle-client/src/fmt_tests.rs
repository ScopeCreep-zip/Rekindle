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
