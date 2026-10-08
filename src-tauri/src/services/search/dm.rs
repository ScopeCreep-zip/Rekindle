//! Search the `dm_messages` FTS5 index — the per-DM ratchet log added
//! in #30. Same shape as channel/thread search: bm25 ranking, ±1
//! message context.

use rekindle_types::search::{MessageSearch, SearchHit, SearchScope, SearchSort};
use rusqlite::Connection;

pub fn search_dm_messages_table(
    conn: &Connection,
    owner_key: &str,
    match_expr: &str,
    req: &MessageSearch,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    let order = match req.sort {
        SearchSort::Relevance => "rank ASC, d.timestamp DESC",
        SearchSort::Newest => "d.timestamp DESC",
        SearchSort::Oldest => "d.timestamp ASC",
    };

    let sql = format!(
        "SELECT d.id, d.record_key, d.sender_pseudonym, d.body, d.timestamp, \
                bm25(dm_messages_fts) AS rank \
           FROM dm_messages_fts \
           JOIN dm_messages d ON d.id = dm_messages_fts.rowid \
          WHERE dm_messages_fts MATCH ?1 \
            AND d.owner_key = ?2 \
            AND (?3 IS NULL OR d.record_key = ?3) \
            AND (?4 IS NULL OR d.sender_pseudonym = ?4) \
            AND (?5 IS NULL OR d.timestamp < ?5) \
            AND (?6 IS NULL OR d.timestamp > ?6) \
          ORDER BY {order} \
          LIMIT ?7 OFFSET ?8"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![
            match_expr,
            owner_key,
            req.filters.in_channel,
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
                row.get::<_, i64>(4)?,
                row.get::<_, f64>(5)?,
            ))
        },
    )?;

    let mut hits: Vec<SearchHit> = Vec::new();
    for row in rows {
        let (id, record_key, sender, body, ts, rank) = row?;
        let (before_body, after_body) = fetch_dm_context(conn, owner_key, &record_key, ts, id);
        hits.push(SearchHit {
            scope: SearchScope::Dm,
            conversation_id: record_key,
            message_id: None,
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

fn fetch_dm_context(
    conn: &Connection,
    owner_key: &str,
    record_key: &str,
    ts: i64,
    id: i64,
) -> (Option<String>, Option<String>) {
    let before: Option<String> = conn
        .query_row(
            "SELECT body FROM dm_messages \
              WHERE owner_key = ?1 AND record_key = ?2 AND timestamp < ?3 AND id != ?4 \
              ORDER BY timestamp DESC LIMIT 1",
            rusqlite::params![owner_key, record_key, ts, id],
            |r| r.get(0),
        )
        .ok();
    let after: Option<String> = conn
        .query_row(
            "SELECT body FROM dm_messages \
              WHERE owner_key = ?1 AND record_key = ?2 AND timestamp > ?3 AND id != ?4 \
              ORDER BY timestamp ASC LIMIT 1",
            rusqlite::params![owner_key, record_key, ts, id],
            |r| r.get(0),
        )
        .ok();
    (before, after)
}
