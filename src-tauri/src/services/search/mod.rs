//! Architecture §23 — local FTS5 search across all locally-stored
//! decrypted messages. The schema is defined in `001_init.sql` (three
//! external-content FTS5 virtual tables: `messages_fts`,
//! `thread_messages_fts`, `dm_messages_fts`).
//!
//! This module owns the orchestration: clamp the limit, sanitize the
//! query into a MATCH expression, route across scopes, merge + rank,
//! attach ±1 context.

mod context;
mod dm;
mod messages;
mod query;
mod threads;

#[cfg(test)]
mod tests;

use rekindle_types::search::{
    MessageSearch, SearchHit, SearchResult, SearchSort, MAX_SEARCH_LIMIT,
};

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

pub async fn search_messages(
    state: &SharedState,
    pool: &DbPool,
    mut req: MessageSearch,
) -> Result<SearchResult, String> {
    let owner_key = state_helpers::current_owner_key(state)?;

    req.limit = req.limit.clamp(1, MAX_SEARCH_LIMIT);

    let Some(match_expr) = query::build_match_expr(&req.query) else {
        return Ok(SearchResult::default());
    };

    let started = std::time::Instant::now();
    let req_clone = req.clone();
    let owner_key_owned = owner_key.clone();
    let match_owned = match_expr.clone();

    let hits = db_call(pool, move |conn| {
        let mut combined: Vec<SearchHit> = Vec::new();
        // `in_thread` filter routes only to thread search; otherwise we
        // search messages + dm + threads and merge.
        if req_clone.filters.in_thread.is_some() {
            let h = threads::search_thread_messages_table(
                conn,
                &owner_key_owned,
                &match_owned,
                &req_clone,
            )?;
            combined.extend(h);
        } else {
            let m =
                messages::search_messages_table(conn, &owner_key_owned, &match_owned, &req_clone)?;
            combined.extend(m);
            let d = dm::search_dm_messages_table(conn, &owner_key_owned, &match_owned, &req_clone)?;
            combined.extend(d);
            let t = threads::search_thread_messages_table(
                conn,
                &owner_key_owned,
                &match_owned,
                &req_clone,
            )?;
            combined.extend(t);
        }
        Ok(combined)
    })
    .await?;

    let mut hits = hits;
    sort_combined(&mut hits, req.sort);
    hits.truncate(req.limit as usize);

    let total_returned = u32::try_from(hits.len()).unwrap_or(u32::MAX);
    let elapsed = started.elapsed();
    let query_ms = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);

    Ok(SearchResult {
        hits,
        total_returned,
        query_ms,
    })
}

fn sort_combined(hits: &mut [SearchHit], sort: SearchSort) {
    match sort {
        SearchSort::Relevance => hits.sort_by(|a, b| {
            b.rank
                .partial_cmp(&a.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.timestamp.cmp(&a.timestamp))
        }),
        SearchSort::Newest => hits.sort_by(|a, b| b.timestamp.cmp(&a.timestamp)),
        SearchSort::Oldest => hits.sort_by(|a, b| a.timestamp.cmp(&b.timestamp)),
    }
}
