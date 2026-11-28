//! Architecture §24.1 — local-only community analytics facade.
//!
//! Pure SQL aggregations now live in the `rekindle-analytics` crate.
//! This module wraps the crate's `&Connection`-taking compute fns with
//! `Db` + AppState/owner-key resolution + voice/member event
//! loggers (db_fire fire-and-forget inserts).

use rekindle_types::analytics::CommunityAnalytics;

use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;

pub async fn compute_community_analytics(
    state: &SharedState,
    pool: &Db,
    community_id: &str,
) -> Result<CommunityAnalytics, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id = community_id.to_string();
    let started = std::time::Instant::now();
    let now = rekindle_utils::timestamp_ms_i64();

    let owner_for_db = owner_key.clone();
    let cid_for_db = community_id.clone();
    let analytics = db_call(pool, move |conn| {
        let members =
            rekindle_analytics::member_metrics::compute(conn, &owner_for_db, &cid_for_db, now);
        let channels =
            rekindle_analytics::channel_metrics::compute(conn, &owner_for_db, &cid_for_db, now)?;
        let growth = rekindle_analytics::growth::compute(conn, &owner_for_db, &cid_for_db, now);
        let activity_by_hour =
            rekindle_analytics::activity_by_hour::compute(conn, &owner_for_db, &cid_for_db, now);
        let storage_usage = rekindle_analytics::storage::compute(conn, &owner_for_db, &cid_for_db);
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
