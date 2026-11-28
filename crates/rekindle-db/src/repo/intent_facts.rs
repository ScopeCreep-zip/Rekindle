//! Counts an intent review states (plan U1.4; ADR 0019): the history a
//! deletion destroys, the audience a message reaches, and the work a
//! logout or lock leaves unsent.

use rusqlite::{params, Connection};

fn count(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<u32> {
    let n: i64 = conn.query_row(sql, args, |row| row.get(0))?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Messages stored for a channel: what deleting it destroys.
///
/// # Errors
/// The query failed.
pub fn channel_messages(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
) -> rusqlite::Result<u32> {
    count(
        conn,
        "SELECT COUNT(*) FROM messages \
         WHERE owner_key = ?1 AND conversation_type = 'channel' AND conversation_id = ?2",
        params![owner_key, channel_id],
    )
}

/// Members we know of in a community.
///
/// # Errors
/// The query failed.
pub fn community_members(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<u32> {
    count(
        conn,
        "SELECT COUNT(*) FROM community_members WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )
}

/// Sends the outbox has not settled (plan E2.5, D22; the record writes a
/// lock left unsent are its `dht_write` rows): what a logout or lock
/// leaves undelivered.
///
/// # Errors
/// The query failed.
pub fn unsent(conn: &Connection, owner_key: &str) -> rusqlite::Result<u32> {
    let sends = super::outbox::unsettled_count(conn, owner_key)?;
    Ok(u32::try_from(sends).unwrap_or(u32::MAX))
}

#[cfg(test)]
#[path = "intent_facts_tests.rs"]
mod tests;
