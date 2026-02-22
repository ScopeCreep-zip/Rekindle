pub mod time;

// Re-export at crate root for convenience: `rekindle_utils::timestamp_ms()`
pub use time::{timestamp_ms, timestamp_ms_i64, timestamp_secs, timestamp_secs_i64};
