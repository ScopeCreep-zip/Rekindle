//! Architecture §32 Phase 7 Week 23 — "peak activity hours" histogram.
//!
//! Bucket every message in the last 30 days by its UTC hour-of-day
//! (`(timestamp / 3_600_000) % 24` with the timestamp in ms) and
//! return the 24-element count array. Same window as the daily
//! timeseries so the two visualizations agree.

use rekindle_types::analytics::ActivityByHour;
use rusqlite::Connection;

use super::buckets::window_start_ms;

const ONE_HOUR_MS: i64 = 60 * 60 * 1000;

pub fn compute(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> ActivityByHour {
    let window_start = window_start_ms(now_ms);
    let Ok(mut stmt) = conn.prepare(
        "SELECT (m.timestamp / ?1) % 24 AS hour, COUNT(*) AS n \
           FROM messages m \
           JOIN channels c ON c.owner_key = m.owner_key AND c.id = m.conversation_id \
          WHERE m.owner_key = ?2 AND c.community_id = ?3 AND m.timestamp >= ?4 \
       GROUP BY hour",
    ) else {
        return ActivityByHour::default();
    };
    let rows = stmt.query_map(
        rusqlite::params![ONE_HOUR_MS, owner_key, community_id, window_start],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?)),
    );
    let Ok(rows) = rows else {
        return ActivityByHour::default();
    };
    let mut hour_counts = [0u32; 24];
    for row in rows.filter_map(Result::ok) {
        let (hour, count) = row;
        // SQL `% 24` already pins the value to 0..23, so the cast is
        // safe — but we go through TryFrom to convince clippy without
        // an `#[allow]`.
        if let Ok(idx) = usize::try_from(hour) {
            if idx < 24 {
                hour_counts[idx] = count;
            }
        }
    }
    ActivityByHour { hour_counts }
}

#[cfg(test)]
#[path = "activity_by_hour_tests.rs"]
mod tests;
