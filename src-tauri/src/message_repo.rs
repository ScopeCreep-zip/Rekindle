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
    insert_channel_message_with_id(
        conn,
        owner_key,
        channel_id,
        sender_key,
        body,
        timestamp,
        is_read,
        mek_generation,
        None,
        false,
    )
}

/// Insert a channel message with an optional protocol message ID.
pub fn insert_channel_message_with_id(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel_id: &str,
    sender_key: &str,
    body: &str,
    timestamp: i64,
    is_read: bool,
    mek_generation: Option<i64>,
    message_id: Option<&str>,
    automod_blurred: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, automod_blurred, timestamp, is_read, mek_generation, message_id) \
         VALUES (?, ?, 'channel', ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            owner_key,
            channel_id,
            sender_key,
            body,
            i32::from(automod_blurred),
            timestamp,
            is_read,
            mek_generation,
            message_id
        ],
    )?;
    Ok(())
}

/// Insert a channel message with protocol metadata used for deterministic ordering.
pub fn insert_channel_message_with_protocol_metadata(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel_id: &str,
    sender_key: &str,
    body: &str,
    timestamp: i64,
    is_read: bool,
    mek_generation: Option<i64>,
    message_id: &str,
    lamport_ts: u64,
    automod_blurred: bool,
) -> Result<(), rusqlite::Error> {
    let lamport_ts = i64::try_from(lamport_ts).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO messages \
         (owner_key, conversation_id, conversation_type, sender_key, body, automod_blurred, timestamp, is_read, \
          mek_generation, message_id, lamport_ts) \
         VALUES (?, ?, 'channel', ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            owner_key,
            channel_id,
            sender_key,
            body,
            i32::from(automod_blurred),
            timestamp,
            is_read,
            mek_generation,
            message_id,
            lamport_ts,
        ],
    )?;
    Ok(())
}
