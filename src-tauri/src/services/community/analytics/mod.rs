//! Architecture §24.1 — local-only community analytics.
//!
//! All metrics are pure SQL aggregations against tables this device
//! already owns; nothing leaves the device. Permission gate
//! [`rekindle_types::permissions::VIEW_INSIGHTS`] is enforced at the
//! command layer (`commands/community/analytics.rs`).

mod activity_by_hour;
mod buckets;
mod channel_metrics;
mod growth;
mod member_metrics;
mod storage;

#[cfg(test)]
mod tests;

use rekindle_types::analytics::CommunityAnalytics;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

const SEVEN_DAYS_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const THIRTY_DAYS_MS: i64 = 30 * 24 * 60 * 60 * 1000;
pub(crate) const ONE_DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Number of daily buckets shipped in every timeseries — matches the
/// 30-day growth sample window so the UI can render all metrics on
/// the same x-axis.
pub(crate) const DAILY_TIMESERIES_DAYS: u32 = 30;

pub async fn compute_community_analytics(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
) -> Result<CommunityAnalytics, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id = community_id.to_string();
    let started = std::time::Instant::now();
    let now = rekindle_utils::timestamp_ms_i64();

    let owner_for_db = owner_key.clone();
    let cid_for_db = community_id.clone();
    let analytics = db_call(pool, move |conn| {
        let members = member_metrics::compute(conn, &owner_for_db, &cid_for_db, now);
        let channels = channel_metrics::compute(conn, &owner_for_db, &cid_for_db, now)?;
        let growth = growth::compute(conn, &owner_for_db, &cid_for_db, now);
        let activity_by_hour =
            activity_by_hour::compute(conn, &owner_for_db, &cid_for_db, now);
        let storage_usage = storage::compute(conn, &owner_for_db, &cid_for_db);
        Ok(CommunityAnalytics {
            community_id: cid_for_db.clone(),
            members,
            channels,
            growth,
            activity_by_hour,
            storage_usage,
            computed_in_ms: 0,
        })
    })
    .await?;

    let elapsed = started.elapsed();
    let mut analytics = analytics;
    analytics.computed_in_ms = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
    Ok(analytics)
}

/// Append a "join" event to the voice session log. Caller passes the
/// current monotonic timestamp; this is fire-and-forget — analytics
/// can tolerate dropped log entries.
pub fn log_voice_join(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    pseudonym: &str,
) {
    log_voice_event(pool, owner_key, community_id, channel_id, pseudonym, "join");
}

pub fn log_voice_leave(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    pseudonym: &str,
) {
    log_voice_event(pool, owner_key, community_id, channel_id, pseudonym, "leave");
}

fn log_voice_event(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    pseudonym: &str,
    event_type: &'static str,
) {
    let owner = owner_key.to_string();
    let cid = community_id.to_string();
    let chid = channel_id.to_string();
    let pse = pseudonym.to_string();
    let ts = rekindle_utils::timestamp_ms_i64();
    crate::db_helpers::db_fire(pool, "voice session event", move |conn| {
        conn.execute(
            "INSERT INTO voice_session_events \
             (owner_key, community_id, channel_id, member_pseudonym, event_type, occurred_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![owner, cid, chid, pse, event_type, ts],
        )?;
        Ok(())
    });
}

/// Append a member-leave event to the analytics log.
pub fn log_member_leave(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    pseudonym: &str,
) {
    let owner = owner_key.to_string();
    let cid = community_id.to_string();
    let pse = pseudonym.to_string();
    let ts = rekindle_utils::timestamp_ms_i64();
    crate::db_helpers::db_fire(pool, "member leave event", move |conn| {
        conn.execute(
            "INSERT INTO community_member_leaves \
             (owner_key, community_id, pseudonym_key, left_at) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![owner, cid, pse, ts],
        )?;
        Ok(())
    });
}
