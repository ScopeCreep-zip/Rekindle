//! Display helpers the views share: the frontends' formatters and the
//! untrusted-text sanitizer, under one import.

pub use rekindle_client::fmt::{
    abbreviate_key, format_duration_ago, format_time_short, format_timestamp, format_uptime,
};
pub use rekindle_utils::text::sanitize_for_display;
