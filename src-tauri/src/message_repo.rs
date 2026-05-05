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
    insert_channel_message_with_full_metadata(
        conn,
        owner_key,
        channel_id,
        sender_key,
        body,
        timestamp,
        is_read,
        mek_generation,
        message_id,
        lamport_ts,
        automod_blurred,
        None,
    )
}

/// Insert a channel message including the optional `forwarded_from_author`
/// attribution (set when the row originates from a `ChannelEntry::Forward`).
#[allow(clippy::too_many_arguments)]
pub fn insert_channel_message_with_full_metadata(
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
    forwarded_from_author: Option<&str>,
) -> Result<(), rusqlite::Error> {
    insert_channel_message_full(
        conn,
        owner_key,
        channel_id,
        sender_key,
        body,
        timestamp,
        is_read,
        mek_generation,
        message_id,
        lamport_ts,
        automod_blurred,
        forwarded_from_author,
        0,
        None,
    )
}

/// Insert a channel message with all metadata including `flags` (Lost
/// Cargo VOICE_MESSAGE / SUPPRESS_NOTIFICATIONS / etc.) and an optional
/// pre-serialized `attachment_json` payload. This is the maximal-form
/// helper used by the Lost Cargo upload + voice-message paths;
/// thinner wrappers above call into it with sensible defaults.
#[allow(clippy::too_many_arguments)]
pub fn insert_channel_message_full(
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
    forwarded_from_author: Option<&str>,
    flags: u32,
    attachment_json: Option<&str>,
) -> Result<(), rusqlite::Error> {
    let lamport_ts = i64::try_from(lamport_ts).unwrap_or(i64::MAX);
    let flags = i64::from(flags);
    conn.execute(
        "INSERT INTO messages \
         (owner_key, conversation_id, conversation_type, sender_key, body, automod_blurred, timestamp, is_read, \
          mek_generation, message_id, lamport_ts, forwarded_from_author, flags, attachment_json) \
         VALUES (?, ?, 'channel', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            forwarded_from_author,
            flags,
            attachment_json,
        ],
    )?;
    Ok(())
}

/// Look up a single channel message by its server message id (for forwarding).
/// Returns None if the message is not in the local cache (forwarder never
/// fetches from DHT — the source must already be in their SQLite).
pub fn find_channel_message_by_id(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> Result<Option<ChannelMessageRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT sender_key, body FROM messages \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND message_id = ?3 LIMIT 1",
    )?;
    let mut rows = stmt.query(rusqlite::params![owner_key, channel_id, message_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(ChannelMessageRow {
            sender_key: row.get(0)?,
            body: row.get(1)?,
        }))
    } else {
        Ok(None)
    }
}

/// Minimal row returned by `find_channel_message_by_id` — only the fields
/// the forward path needs (sender attribution + plaintext body to re-encrypt).
pub struct ChannelMessageRow {
    pub sender_key: String,
    pub body: String,
}
