//! Local full-text search over the stored messages (architecture §23):
//! the two external-content FTS5 tables of `001_init.sql`, `messages_fts`
//! and `thread_messages_fts`. [`search`] sanitizes the query into a MATCH
//! expression, routes the scopes, merges and ranks the hits, and attaches
//! the ±1 context.

mod context;
mod messages;
mod query;
mod threads;

#[cfg(test)]
mod tests;

use rekindle_types::search::{MessageSearch, SearchHit, SearchSort, MAX_SEARCH_LIMIT};
use rusqlite::Connection;

/// `owner_key`'s hits for `req`, ranked by its sort, at most its limit
/// (clamped to `1..=MAX_SEARCH_LIMIT`); a blank query finds nothing.
///
/// # Errors
/// A query failed.
pub fn search(
    conn: &Connection,
    owner_key: &str,
    req: &MessageSearch,
) -> rusqlite::Result<Vec<SearchHit>> {
    let mut req = req.clone();
    req.limit = req.limit.clamp(1, MAX_SEARCH_LIMIT);
    let Some(match_expr) = query::build_match_expr(&req.query) else {
        return Ok(Vec::new());
    };
    // An `in_thread` filter searches the thread only; otherwise the
    // channel and DM messages and the threads are merged.
    let mut hits = Vec::new();
    if req.filters.in_thread.is_none() {
        hits.extend(messages::search_messages_table(
            conn,
            owner_key,
            &match_expr,
            &req,
        )?);
    }
    hits.extend(threads::search_thread_messages_table(
        conn,
        owner_key,
        &match_expr,
        &req,
    )?);
    sort_combined(&mut hits, req.sort);
    hits.truncate(req.limit as usize);
    Ok(hits)
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
