//! Message persistence helpers.
//!
//! Pure `rusqlite` functions that encapsulate SQL for the `messages` table.
//! Callers wrap these in `db_call` or `db_fire` as appropriate.

/// Insert a direct message.
pub fn insert_dm(
    conn: &rusqlite::Connection,
    owner_key: &str,
    peer_key: &str,
    sender_key: &str,
    body: &str,
    timestamp: i64,
    is_read: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read) \
         VALUES (?, ?, 'dm', ?, ?, ?, ?)",
        rusqlite::params![owner_key, peer_key, sender_key, body, timestamp, is_read],
    )?;
    Ok(())
}

/// Insert a channel message (community or legacy plaintext).
#[allow(clippy::too_many_arguments)]
pub fn insert_channel_message(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel_id: &str,
    sender_key: &str,
    body: &str,
    timestamp: i64,
    is_read: bool,
    mek_generation: Option<i64>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
         VALUES (?, ?, 'channel', ?, ?, ?, ?, ?)",
        rusqlite::params![owner_key, channel_id, sender_key, body, timestamp, is_read, mek_generation],
    )?;
    Ok(())
}
