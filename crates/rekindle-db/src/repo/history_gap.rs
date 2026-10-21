//! History-gap markers (plan E2.5 D23; groups E2.6): the local row a
//! conversation shows where a writer's chain was retired before we read it.

use rusqlite::{params, Connection};

/// Store the local marker of a history gap (plan E2.5, D23; groups E2.6):
/// `writer`'s chain entries through `through_seq` were retired before we
/// fetched them. For a DM the conversation is the peer and the writer the
/// peer; for a group the conversation is the group's id and the writer the
/// member whose chain retired. It is never sent: an empty body from the
/// writer, read and `delivered`. A second marker for the same
/// `(conversation, writer, through_seq)` is not stored; the held row's id
/// is returned.
///
/// # Errors
/// `through_seq` is not a stored seq (0 or above `i64::MAX`), or the write
/// failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    conversation: (&str, rekindle_types::message::ConversationType),
    writer: &str,
    through_seq: u64,
    timestamp: i64,
) -> rusqlite::Result<i64> {
    let through = i64::try_from(through_seq)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let (conversation_id, kind) = (conversation.0, conversation.1.as_str());
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, is_read, send_state, history_gap_through) \
         SELECT ?1, ?2, ?3, ?4, '', ?5, 1, 'delivered', ?6 \
         WHERE NOT EXISTS (SELECT 1 FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
                           AND conversation_type = ?3 AND sender_key = ?4 \
                           AND history_gap_through = ?6)",
        params![owner_key, conversation_id, kind, writer, timestamp, through],
    )?;
    conn.query_row(
        "SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = ?3 AND sender_key = ?4 AND history_gap_through = ?5",
        params![owner_key, conversation_id, kind, writer, through],
        |row| row.get(0),
    )
}
