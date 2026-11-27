//! Per-day bucketing helpers for the analytics module.
//!
//! Architecture §32 Phase 7 Week 23 expects "messages per channel per
//! day" and "active member count per day" timeseries. The shape is the
//! same for both — a `Vec<DailySample>` covering the most recent
//! `DAILY_TIMESERIES_DAYS` days, oldest first, with day_unix_ms set to
//! midnight UTC of each bucket.
//!
//! These helpers handle the bucket-skeleton construction + result-row
//! merging so the per-metric SQL stays focused on the aggregation
//! itself (counts, distinct counts, etc.).

use rekindle_types::analytics::{DailySample, DailyTimeseries};

use super::{DAILY_TIMESERIES_DAYS, ONE_DAY_MS};

/// Return the midnight-UTC ms for the bucket containing `unix_ms`.
pub fn day_floor_ms(unix_ms: i64) -> i64 {
    unix_ms - unix_ms.rem_euclid(ONE_DAY_MS)
}

/// Build the empty 30-bucket skeleton ending at the bucket containing
/// `now_ms` (inclusive), oldest first.
pub fn empty_skeleton(now_ms: i64) -> Vec<DailySample> {
    let today_midnight = day_floor_ms(now_ms);
    let mut out = Vec::with_capacity(DAILY_TIMESERIES_DAYS as usize);
    for offset in (0..i64::from(DAILY_TIMESERIES_DAYS)).rev() {
        out.push(DailySample {
            day_unix_ms: today_midnight - offset * ONE_DAY_MS,
            value: 0,
        });
    }
    out
}

/// Merge a list of `(day_unix_ms, value)` results from a SQL query
/// into the empty skeleton. Rows whose day falls outside the window
/// are silently dropped.
pub fn merge_into_skeleton(
    mut skeleton: Vec<DailySample>,
    rows: impl IntoIterator<Item = (i64, u32)>,
) -> DailyTimeseries {
    use std::collections::HashMap;
    let by_day: HashMap<i64, u32> = rows.into_iter().collect();
    for sample in &mut skeleton {
        if let Some(value) = by_day.get(&sample.day_unix_ms) {
            sample.value = *value;
        }
    }
    DailyTimeseries { samples: skeleton }
}

/// Earliest day included in the 30-day window for `now_ms` —
/// callers use this as the SQL `WHERE timestamp >= ?` lower bound.
pub fn window_start_ms(now_ms: i64) -> i64 {
    day_floor_ms(now_ms) - i64::from(DAILY_TIMESERIES_DAYS - 1) * ONE_DAY_MS
}

#[cfg(test)]
#[path = "buckets_tests.rs"]
mod tests;
