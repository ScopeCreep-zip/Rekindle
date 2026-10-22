//! A channel's pinned messages (architecture §16.4), as each peer accepted
//! the pins off the mesh and its own.

use rusqlite::{params, Connection};

/// One pinned message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRow {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
}

/// Store a pin; a message already pinned keeps its first pin.
///
/// # Errors
/// The insert failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pin: &PinRow,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO channel_pins \
         (owner_key, community_id, channel_id, message_id, pinned_by, pinned_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            owner_key,
            community_id,
            pin.channel_id,
            pin.message_id,
            pin.pinned_by,
            pin.pinned_at.cast_signed()
        ],
    )?;
    Ok(n > 0)
}

/// Drop the pin of `message_id`; `true` when there was one.
///
/// # Errors
/// The delete failed.
pub fn delete(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM channel_pins WHERE owner_key = ?1 AND community_id = ?2 \
         AND channel_id = ?3 AND message_id = ?4",
        params![owner_key, community_id, channel_id, message_id],
    )?;
    Ok(n > 0)
}

/// The channel's pins, oldest first.
///
/// # Errors
/// The query failed.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
) -> rusqlite::Result<Vec<PinRow>> {
    let mut stmt = conn.prepare(
        "SELECT message_id, channel_id, pinned_by, pinned_at FROM channel_pins \
         WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3 \
         ORDER BY pinned_at, message_id",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id, channel_id], |row| {
        Ok(PinRow {
            message_id: row.get(0)?,
            channel_id: row.get(1)?,
            pinned_by: row.get(2)?,
            pinned_at: row.get::<_, i64>(3)?.cast_unsigned(),
        })
    })?;
    rows.collect()
}
