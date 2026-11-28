//! ±1 message context lookup for `messages` hits.
//!
//! Architecture §23.2 line 2698: "Results include ±1 message context."
//! Each hit includes the body of the immediately preceding and following
//! message in the same conversation so the UI can render a snippet
//! without an extra round-trip.

use rusqlite::Connection;

pub fn fetch_adjacent_context(
    conn: &Connection,
    owner_key: &str,
    conversation_id: &str,
    timestamp: i64,
    id: i64,
) -> (Option<String>, Option<String>) {
    let before: Option<String> = conn
        .query_row(
            "SELECT body FROM messages \
              WHERE owner_key = ?1 AND conversation_id = ?2 AND timestamp < ?3 AND id != ?4 \
              ORDER BY timestamp DESC LIMIT 1",
            rusqlite::params![owner_key, conversation_id, timestamp, id],
            |r| r.get(0),
        )
        .ok();
    let after: Option<String> = conn
        .query_row(
            "SELECT body FROM messages \
              WHERE owner_key = ?1 AND conversation_id = ?2 AND timestamp > ?3 AND id != ?4 \
              ORDER BY timestamp ASC LIMIT 1",
            rusqlite::params![owner_key, conversation_id, timestamp, id],
            |r| r.get(0),
        )
        .ok();
    (before, after)
}
