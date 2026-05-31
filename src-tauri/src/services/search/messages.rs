//! Search the `messages` table (channel + DM-bridge messages).
//!
//! External-content FTS5 design — see `001_init.sql` `messages_fts`.
//! `bm25(messages_fts)` returns the BM25 rank where lower is better; we
//! flip the sign so callers see "higher = better".

use rekindle_types::search::{HasFilter, MessageSearch, SearchHit, SearchScope, SearchSort};
use rusqlite::Connection;

use super::context::fetch_adjacent_context;

/// Run the FTS5 search against `messages` and return ranked hits with
/// ±1-message context attached. `match_expr` is the already-sanitized
/// FTS5 MATCH expression.
pub fn search_messages_table(
    conn: &Connection,
    owner_key: &str,
    match_expr: &str,
    req: &MessageSearch,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    let order = order_clause(req.sort);
    let has_filter = req.filters.has.first().copied().map(has_to_pattern);
    let mention_pattern = req.filters.mentions.as_ref().map(|pk| format!("%<@{pk}>%"));

    // Architecture §32 Phase 7 W23 line 4111 — community-scoped search.
    // The `messages` table stores `conversation_id` (channel UUID) but
    // no `community_id` column; the relation lives on `channels.community_id`.
    // Filter via an EXISTS subquery so the channels join only fires when
    // an `in_community` filter is present.
    let sql = format!(
        "SELECT m.id, m.conversation_id, m.message_id, m.sender_key, m.body, m.timestamp, \
                bm25(messages_fts) AS rank \
           FROM messages_fts \
           JOIN messages m ON m.id = messages_fts.rowid \
          WHERE messages_fts MATCH ?1 \
            AND m.owner_key = ?2 \
            AND (?3 IS NULL OR m.conversation_id = ?3) \
            AND (?4 IS NULL OR m.sender_key = ?4) \
            AND (?5 IS NULL OR m.timestamp < ?5) \
            AND (?6 IS NULL OR m.timestamp > ?6) \
            AND (?7 IS NULL OR m.attachment_json LIKE ?7) \
            AND (?8 IS NULL OR m.body LIKE ?8) \
            AND (?9 IS NULL OR EXISTS (\
                  SELECT 1 FROM channel_pins cp \
                   WHERE cp.owner_key = m.owner_key \
                     AND cp.channel_id = m.conversation_id \
                     AND cp.message_id = m.message_id)) \
            AND (?12 IS NULL OR EXISTS (\
                  SELECT 1 FROM channels ch \
                   WHERE ch.owner_key = m.owner_key \
                     AND ch.id = m.conversation_id \
                     AND ch.community_id = ?12)) \
          ORDER BY {order} \
          LIMIT ?10 OFFSET ?11"
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
            has_filter,
            mention_pattern,
            req.filters.is_pinned.map(i64::from),
            i64::from(req.limit),
            i64::from(req.offset),
            req.filters.in_community,
        ],
        |row| {
            Ok(RowData {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                message_id: row.get::<_, Option<String>>(2)?,
                sender_key: row.get(3)?,
                body: row.get(4)?,
                timestamp: row.get(5)?,
                rank: row.get(6)?,
            })
        },
    )?;

    let mut hits: Vec<SearchHit> = Vec::new();
    for row in rows {
        let r = row?;
        let (before_body, after_body) =
            fetch_adjacent_context(conn, owner_key, &r.conversation_id, r.timestamp, r.id);
        hits.push(SearchHit {
            scope: SearchScope::Channel,
            conversation_id: r.conversation_id,
            message_id: r.message_id,
            sender_key: r.sender_key,
            body: r.body,
            timestamp: r.timestamp,
            // bm25 returns "lower is better"; flip sign so callers see
            // "higher = better" without having to know FTS5 internals.
            rank: -r.rank,
            before_body,
            after_body,
        });
    }
    Ok(hits)
}

struct RowData {
    id: i64,
    conversation_id: String,
    message_id: Option<String>,
    sender_key: String,
    body: String,
    timestamp: i64,
    rank: f64,
}

fn order_clause(sort: SearchSort) -> &'static str {
    match sort {
        SearchSort::Relevance => "rank ASC, m.timestamp DESC",
        SearchSort::Newest => "m.timestamp DESC",
        SearchSort::Oldest => "m.timestamp ASC",
    }
}

fn has_to_pattern(has: HasFilter) -> String {
    let kind = match has {
        HasFilter::Link => "link",
        HasFilter::File => "file",
        HasFilter::Image => "image",
        HasFilter::Video => "video",
        HasFilter::Embed => "embed",
        HasFilter::Poll => "poll",
        HasFilter::VoiceMessage => "voice_message",
    };
    format!("%\"kind\":\"{kind}\"%")
}
