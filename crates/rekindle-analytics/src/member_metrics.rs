//! Member-side aggregates: total, active windows, joins, leaves, retention.
//!
//! "Active" means "has sent at least one message in the window" — the
//! cheapest signal we have without per-member presence telemetry.

use rekindle_types::analytics::{DailyTimeseries, MemberMetrics};
use rusqlite::Connection;

use super::buckets::{day_floor_ms, empty_skeleton, merge_into_skeleton, window_start_ms};
use super::{ONE_DAY_MS, SEVEN_DAYS_MS, THIRTY_DAYS_MS};

pub fn compute(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> MemberMetrics {
    let total_members: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM community_members \
              WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let active_7d =
        active_distinct_senders(conn, owner_key, community_id, now_ms - SEVEN_DAYS_MS);
    let active_30d =
        active_distinct_senders(conn, owner_key, community_id, now_ms - THIRTY_DAYS_MS);

    let joins_7d: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM community_members \
              WHERE owner_key = ?1 AND community_id = ?2 AND joined_at > ?3",
            rusqlite::params![owner_key, community_id, now_ms - SEVEN_DAYS_MS],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let leaves_7d: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM community_member_leaves \
              WHERE owner_key = ?1 AND community_id = ?2 AND left_at > ?3",
            rusqlite::params![owner_key, community_id, now_ms - SEVEN_DAYS_MS],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let retention_7_of_30 = compute_retention(conn, owner_key, community_id, now_ms);
    let active_per_day = active_per_day_series(conn, owner_key, community_id, now_ms);
    let joins_per_day = joins_per_day_series(conn, owner_key, community_id, now_ms);
    let leaves_per_day = leaves_per_day_series(conn, owner_key, community_id, now_ms);

    MemberMetrics {
        total_members,
        active_7d,
        active_30d,
        joins_7d,
        leaves_7d,
        retention_7_of_30,
        active_per_day,
        joins_per_day,
        leaves_per_day,
    }
}

fn active_per_day_series(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> DailyTimeseries {
    let window_start = window_start_ms(now_ms);
    let Ok(mut stmt) = conn.prepare(
        "SELECT m.timestamp - (m.timestamp % ?1) AS day, \
                COUNT(DISTINCT m.sender_key) AS n \
           FROM messages m \
           JOIN channels c ON c.owner_key = m.owner_key AND c.id = m.conversation_id \
          WHERE m.owner_key = ?2 AND c.community_id = ?3 AND m.timestamp >= ?4 \
       GROUP BY day",
    ) else {
        return DailyTimeseries::default();
    };
    let rows = stmt
        .query_map(
            rusqlite::params![ONE_DAY_MS, owner_key, community_id, window_start],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?)),
        );
    let Ok(rows) = rows else {
        return DailyTimeseries::default();
    };
    let collected: Vec<(i64, u32)> = rows
        .filter_map(Result::ok)
        .filter(|(day, _)| *day <= day_floor_ms(now_ms))
        .collect();
    merge_into_skeleton(empty_skeleton(now_ms), collected)
}

fn joins_per_day_series(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> DailyTimeseries {
    let window_start = window_start_ms(now_ms);
    let Ok(mut stmt) = conn.prepare(
        "SELECT joined_at - (joined_at % ?1) AS day, COUNT(*) AS n \
           FROM community_members \
          WHERE owner_key = ?2 AND community_id = ?3 AND joined_at >= ?4 \
       GROUP BY day",
    ) else {
        return DailyTimeseries::default();
    };
    let rows = stmt
        .query_map(
            rusqlite::params![ONE_DAY_MS, owner_key, community_id, window_start],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?)),
        );
    let Ok(rows) = rows else {
        return DailyTimeseries::default();
    };
    let collected: Vec<(i64, u32)> = rows
        .filter_map(Result::ok)
        .filter(|(day, _)| *day <= day_floor_ms(now_ms))
        .collect();
    merge_into_skeleton(empty_skeleton(now_ms), collected)
}

fn leaves_per_day_series(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> DailyTimeseries {
    let window_start = window_start_ms(now_ms);
    let Ok(mut stmt) = conn.prepare(
        "SELECT left_at - (left_at % ?1) AS day, COUNT(*) AS n \
           FROM community_member_leaves \
          WHERE owner_key = ?2 AND community_id = ?3 AND left_at >= ?4 \
       GROUP BY day",
    ) else {
        return DailyTimeseries::default();
    };
    let rows = stmt
        .query_map(
            rusqlite::params![ONE_DAY_MS, owner_key, community_id, window_start],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?)),
        );
    let Ok(rows) = rows else {
        return DailyTimeseries::default();
    };
    let collected: Vec<(i64, u32)> = rows
        .filter_map(Result::ok)
        .filter(|(day, _)| *day <= day_floor_ms(now_ms))
        .collect();
    merge_into_skeleton(empty_skeleton(now_ms), collected)
}

fn active_distinct_senders(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    after_ms: i64,
) -> u32 {
    conn.query_row(
        "SELECT COUNT(DISTINCT m.sender_key) \
           FROM messages m \
           JOIN channels c ON c.owner_key = m.owner_key AND c.id = m.conversation_id \
          WHERE m.owner_key = ?1 AND c.community_id = ?2 AND m.timestamp > ?3",
        rusqlite::params![owner_key, community_id, after_ms],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// Members who joined ≥30 days ago AND posted within the last 7 days,
/// divided by the count who joined ≥30 days ago.
fn compute_retention(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> f64 {
    let joined_30d_cutoff = now_ms - THIRTY_DAYS_MS;
    let active_7d_cutoff = now_ms - SEVEN_DAYS_MS;

    let denom: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM community_members \
              WHERE owner_key = ?1 AND community_id = ?2 AND joined_at <= ?3",
            rusqlite::params![owner_key, community_id, joined_30d_cutoff],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if denom == 0 {
        return 0.0;
    }

    let numer: u32 = conn
        .query_row(
            "SELECT COUNT(DISTINCT cm.pseudonym_key) \
               FROM community_members cm \
               JOIN channels c \
                 ON c.owner_key = cm.owner_key AND c.community_id = cm.community_id \
               JOIN messages m \
                 ON m.owner_key = cm.owner_key \
                AND m.conversation_id = c.id \
                AND m.sender_key = cm.pseudonym_key \
              WHERE cm.owner_key = ?1 \
                AND cm.community_id = ?2 \
                AND cm.joined_at <= ?3 \
                AND m.timestamp > ?4",
            rusqlite::params![owner_key, community_id, joined_30d_cutoff, active_7d_cutoff],
            |r| r.get(0),
        )
        .unwrap_or(0);

    (f64::from(numer) / f64::from(denom)).clamp(0.0, 1.0)
}
