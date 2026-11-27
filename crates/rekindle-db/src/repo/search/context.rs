//! ±1 message context lookup for `messages` hits.
//!
//! Architecture §23.2 line 2698: "Results include ±1 message context."
//! Each hit includes the body of the immediately preceding and following
//! message in the same conversation so the UI can render a snippet
//! without an extra round-trip.

use rusqlite::{params, Connection};

use crate::row::query_one;

pub fn fetch_adjacent_context(
    conn: &Connection,
    owner_key: &str,
    conversation_id: &str,
    timestamp: i64,
    id: i64,
) -> (Option<String>, Option<String>) {
    adjacent_bodies(
        conn,
        [
            "SELECT body FROM messages \
              WHERE owner_key = ?1 AND conversation_id = ?2 AND timestamp < ?3 AND id != ?4 \
              ORDER BY timestamp DESC LIMIT 1",
            "SELECT body FROM messages \
              WHERE owner_key = ?1 AND conversation_id = ?2 AND timestamp > ?3 AND id != ?4 \
              ORDER BY timestamp ASC LIMIT 1",
        ],
        (owner_key, conversation_id, timestamp, id),
    )
}

/// The bodies just before and just after one message: `before_after` bind
/// `?1` the owner, `?2` the conversation or thread, `?3` the message's
/// timestamp and `?4` its row id; a body that cannot be read is `None`.
pub(super) fn adjacent_bodies(
    conn: &Connection,
    before_after: [&str; 2],
    (owner_key, scope, timestamp, id): (&str, &str, i64, i64),
) -> (Option<String>, Option<String>) {
    let [before, after] =
        before_after.map(|sql| query_one(conn, sql, params![owner_key, scope, timestamp, id]).ok());
    (before, after)
}
