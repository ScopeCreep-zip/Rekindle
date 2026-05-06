//! Search the `thread_messages` FTS5 index.
//!
//! Thread messages have their own table per architecture §11; we mirror
//! the same FTS5 + bm25() + adjacent-context shape used for the main
//! `messages` table so search results are uniform across scopes.

use rekindle_types::search::{MessageSearch, SearchHit, SearchScope, SearchSort};
use rusqlite::Connection;

pub fn search_thread_messages_table(
    conn: &Connection,
    owner_key: &str,
    match_expr: &str,
    req: &MessageSearch,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    let order = match req.sort {
        SearchSort::Relevance => "rank ASC, t.timestamp DESC",
        SearchSort::Newest => "t.timestamp DESC",
        SearchSort::Oldest => "t.timestamp ASC",
    };

    // The thread_messages table has community_id but not channel_id —
    // threads are scoped to a community, but the parent channel is
    // recoverable via the threads table join (out of scope here). The
    // `in_channel` filter therefore doesn't apply to thread search;
    // `in_community` is the meaningful scope filter.
    let sql = format!(
        "SELECT t.id, t.community_id, t.thread_id, t.message_id, t.sender_pseudonym, \
                t.body, t.timestamp, bm25(thread_messages_fts) AS rank \
           FROM thread_messages_fts \
           JOIN thread_messages t ON t.id = thread_messages_fts.rowid \
          WHERE thread_messages_fts MATCH ?1 \
            AND t.owner_key = ?2 \
            AND (?3 IS NULL OR t.thread_id = ?3) \
            AND (?4 IS NULL OR t.community_id = ?4) \
            AND (?5 IS NULL OR t.sender_pseudonym = ?5) \
            AND (?6 IS NULL OR t.timestamp < ?6) \
            AND (?7 IS NULL OR t.timestamp > ?7) \
          ORDER BY {order} \
          LIMIT ?8 OFFSET ?9"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![
            match_expr,
            owner_key,
            req.filters.in_thread,
            req.filters.in_community,
            req.filters.from,
            req.filters.before,
            req.filters.after,
            i64::from(req.limit),
            i64::from(req.offset),
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, f64>(7)?,
            ))
        },
    )?;

    let mut hits: Vec<SearchHit> = Vec::new();
    for row in rows {
        let (id, community_id, thread_id, message_id, sender, body, ts, rank) = row?;
        let (before_body, after_body) = fetch_thread_context(conn, owner_key, &thread_id, ts, id);
        hits.push(SearchHit {
            scope: SearchScope::Thread,
            conversation_id: format!("{community_id}/{thread_id}"),
            message_id: Some(message_id),
            sender_key: sender,
            body,
            timestamp: ts,
            rank: -rank,
            before_body,
            after_body,
        });
    }
    Ok(hits)
}

fn fetch_thread_context(
    conn: &Connection,
    owner_key: &str,
    thread_id: &str,
    ts: i64,
    id: i64,
) -> (Option<String>, Option<String>) {
    let before: Option<String> = conn
        .query_row(
            "SELECT body FROM thread_messages \
              WHERE owner_key = ?1 AND thread_id = ?2 AND timestamp < ?3 AND id != ?4 \
              ORDER BY timestamp DESC LIMIT 1",
            rusqlite::params![owner_key, thread_id, ts, id],
            |r| r.get(0),
        )
        .ok();
    let after: Option<String> = conn
        .query_row(
            "SELECT body FROM thread_messages \
              WHERE owner_key = ?1 AND thread_id = ?2 AND timestamp > ?3 AND id != ?4 \
              ORDER BY timestamp ASC LIMIT 1",
            rusqlite::params![owner_key, thread_id, ts, id],
            |r| r.get(0),
        )
        .ok();
    (before, after)
}
