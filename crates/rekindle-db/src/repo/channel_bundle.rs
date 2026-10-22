//! The channel backlog a bootstrap bundle carries (`build_bootstrap_response`).

use rusqlite::{params, Connection};

/// One channel message as a bootstrap bundle carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleMessage {
    pub message_id: String,
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp: i64,
    pub mek_generation: u64,
}

/// The newest `limit` messages of `channel_id` in `community_id`, newest
/// first, for a joiner's bootstrap bundle. A row with no network message id
/// or no stored generation is left out: the joiner dedupes on the id and
/// decrypts under the generation.
///
/// # Errors
/// The query fails.
pub fn channel_bundle_messages(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    limit: i64,
) -> rusqlite::Result<Vec<BundleMessage>> {
    let mut stmt = conn.prepare(
        "SELECT message_id, sender_key, body, timestamp, mek_generation \
         FROM messages \
         WHERE owner_key = ?1 \
           AND conversation_id IN (SELECT id FROM channels WHERE owner_key = ?1 AND community_id = ?2) \
           AND conversation_type = 'channel' AND conversation_id = ?3 \
         ORDER BY timestamp DESC LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id, channel_id, limit], |row| {
        let message_id = row.get::<_, Option<String>>(0)?;
        let mek_generation = row
            .get::<_, Option<i64>>(4)?
            .and_then(|g| u64::try_from(g).ok());
        let (Some(message_id), Some(mek_generation)) = (message_id, mek_generation) else {
            return Ok(None);
        };
        Ok(Some(BundleMessage {
            message_id,
            sender_pseudonym: row.get::<_, String>(1)?,
            body: row.get::<_, String>(2)?,
            timestamp: row.get::<_, i64>(3)?,
            mek_generation,
        }))
    })?;
    rows.filter_map(Result::transpose).collect()
}

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
