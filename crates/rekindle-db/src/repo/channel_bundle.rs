//! A community's channel backlog bounds, for its history catch-up.

use rusqlite::{params, Connection};

/// The oldest Lamport timestamp we hold per channel of a community, for its
/// history catch-up (`0` for a channel whose rows carry none).
///
/// # Errors
/// The query failed.
pub fn oldest_lamport_per_channel(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<std::collections::HashMap<String, u64>> {
    let mut stmt = conn.prepare(
        "SELECT conversation_id, COALESCE(MIN(lamport_ts), 0) \
         FROM messages \
         WHERE owner_key = ?1 \
           AND conversation_id IN \
               (SELECT id FROM channels WHERE owner_key = ?1 AND community_id = ?2) \
           AND conversation_type = 'channel' \
         GROUP BY conversation_id",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id], |row| {
        Ok((row.get::<_, String>(0)?, crate::row::get_u64(row, 1)?))
    })?;
    rows.collect()
}
