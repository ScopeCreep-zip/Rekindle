//! Daily member-count samples for the last 30 days.
//!
//! Computed directly from `community_members.joined_at` minus
//! `community_member_leaves.left_at` — no separate snapshot table
//! needed because the underlying tables are append-only enough that
//! "members at end of day D" is just "joined ≤ D minus left ≤ D".

use rekindle_types::analytics::{GrowthMetrics, GrowthSample};
use rusqlite::Connection;

const SAMPLE_DAYS: u32 = 30;
const ONE_DAY_MS: i64 = 24 * 60 * 60 * 1000;

pub fn compute(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    now_ms: i64,
) -> GrowthMetrics {
    let mut samples = Vec::with_capacity(SAMPLE_DAYS as usize);
    let today_midnight = now_ms - (now_ms.rem_euclid(ONE_DAY_MS));
    for offset in 0..i64::from(SAMPLE_DAYS) {
        let day_end_ms = today_midnight - offset * ONE_DAY_MS + ONE_DAY_MS;
        let joined_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM community_members \
                  WHERE owner_key = ?1 AND community_id = ?2 AND joined_at <= ?3",
                rusqlite::params![owner_key, community_id, day_end_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let left_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM community_member_leaves \
                  WHERE owner_key = ?1 AND community_id = ?2 AND left_at <= ?3",
                rusqlite::params![owner_key, community_id, day_end_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let net = (joined_count - left_count).max(0);
        let member_count = u32::try_from(net).unwrap_or(u32::MAX);
        samples.push(GrowthSample {
            day_unix_ms: today_midnight - offset * ONE_DAY_MS,
            member_count,
        });
    }
    samples.reverse();
    GrowthMetrics { samples }
}
