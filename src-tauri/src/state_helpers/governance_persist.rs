//! The governance clock in SQLite (the merged snapshot is the Host's, plan E3.1 B2c).

use std::sync::Arc;

use crate::state::AppState;

use super::identity::current_owner_key;

/// Raise the persisted governance clock to `clock` (never lowers it).
/// Fire-and-forget: the write is queued on the DB thread ahead of any
/// network write the caller makes with the tick.
pub fn persist_governance_clock(state: &Arc<AppState>, community_id: &str, clock: u64) {
    let Some((_, pool)) = super::node::app_context(state) else {
        return;
    };
    let Ok(owner_key) = current_owner_key(state) else {
        return;
    };
    let Ok(clock) = i64::try_from(clock) else {
        tracing::warn!(community = %community_id, clock, "governance clock beyond i64 — not persisted");
        return;
    };
    let community_id = community_id.to_string();
    crate::db_helpers::db_fire(&pool, "persist governance clock", move |conn| {
        rekindle_db::repo::communities::raise_clock(conn, &owner_key, &community_id, clock)
    });
}
