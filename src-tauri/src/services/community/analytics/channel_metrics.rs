//! Per-channel aggregates + per-day timeseries.
//!
//! Aggregates: messages last 7 days, unique posters last 7 days, peak
//! concurrent voice (replayed from `voice_session_events`). Timeseries
//! (architecture §32 Week 23): messages-per-day and
//! distinct-posters-per-day for the last 30 days.

use rekindle_types::analytics::ChannelMetrics;
use rusqlite::Connection;

use super::buckets::{empty_skeleton, merge_into_skeleton, window_start_ms, day_floor_ms};
use super::{ONE_DAY_MS, SEVEN_DAYS_MS};

pub fn compute(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> Result<Vec<ChannelMetrics>, rusqlite::Error> {
    let after_ms = now_ms - SEVEN_DAYS_MS;
    let window_start = window_start_ms(now_ms);
    let mut stmt = conn.prepare(
        "SELECT id FROM channels \
          WHERE owner_key = ?1 AND community_id = ?2 \
          ORDER BY sort_order ASC",
    )?;
    let channel_ids: Vec<String> = stmt
        .query_map(rusqlite::params![owner_key, community_id], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    let mut out = Vec::with_capacity(channel_ids.len());
    for channel_id in channel_ids {
        let messages_7d: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages \
                  WHERE owner_key = ?1 AND conversation_id = ?2 AND timestamp > ?3",
                rusqlite::params![owner_key, channel_id, after_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let unique_posters_7d: u32 = conn
            .query_row(
                "SELECT COUNT(DISTINCT sender_key) FROM messages \
                  WHERE owner_key = ?1 AND conversation_id = ?2 AND timestamp > ?3",
                rusqlite::params![owner_key, channel_id, after_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let messages_per_day = messages_per_day_series(
            conn, owner_key, &channel_id, now_ms, window_start,
        )?;
        let unique_posters_per_day = unique_posters_per_day_series(
            conn, owner_key, &channel_id, now_ms, window_start,
        )?;

        let peak_concurrent_voice =
            peak_voice_concurrent(conn, owner_key, community_id, &channel_id)?;

        out.push(ChannelMetrics {
            channel_id,
            messages_7d,
            unique_posters_7d,
            peak_concurrent_voice,
            messages_per_day,
            unique_posters_per_day,
        });
    }
    Ok(out)
}

/// Bucket messages by day-floor of `timestamp` and return the merged
/// timeseries.
fn messages_per_day_series(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    now_ms: i64,
    window_start: i64,
) -> Result<rekindle_types::analytics::DailyTimeseries, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT timestamp - (timestamp % ?1) AS day, COUNT(*) AS n \
           FROM messages \
          WHERE owner_key = ?2 AND conversation_id = ?3 AND timestamp >= ?4 \
       GROUP BY day",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![ONE_DAY_MS, owner_key, channel_id, window_start],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?)),
        )?
        .filter_map(Result::ok)
        // Defensive: clip rows whose day_floor accidentally exceeds today.
        .filter(|(day, _)| *day <= day_floor_ms(now_ms));
    Ok(merge_into_skeleton(empty_skeleton(now_ms), rows))
}

fn unique_posters_per_day_series(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    now_ms: i64,
    window_start: i64,
) -> Result<rekindle_types::analytics::DailyTimeseries, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT timestamp - (timestamp % ?1) AS day, \
                COUNT(DISTINCT sender_key) AS n \
           FROM messages \
          WHERE owner_key = ?2 AND conversation_id = ?3 AND timestamp >= ?4 \
       GROUP BY day",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![ONE_DAY_MS, owner_key, channel_id, window_start],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?)),
        )?
        .filter_map(Result::ok)
        .filter(|(day, _)| *day <= day_floor_ms(now_ms));
    Ok(merge_into_skeleton(empty_skeleton(now_ms), rows))
}

/// Replay all `voice_session_events` for `(community, channel)` in
/// chronological order, tracking the running count of joins minus
/// leaves. The maximum value the count reaches is the peak.
fn peak_voice_concurrent(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
) -> Result<u32, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT event_type FROM voice_session_events \
          WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3 \
          ORDER BY occurred_at ASC",
    )?;
    let events = stmt.query_map(
        rusqlite::params![owner_key, community_id, channel_id],
        |r| r.get::<_, String>(0),
    )?;

    let mut current: i32 = 0;
    let mut peak: i32 = 0;
    for ev in events {
        match ev?.as_str() {
            "join" => {
                current += 1;
                if current > peak {
                    peak = current;
                }
            }
            "leave" => {
                if current > 0 {
                    current -= 1;
                }
            }
            _ => {}
        }
    }
    Ok(u32::try_from(peak).unwrap_or(0))
}
