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

/// Remove `writer`'s gap markers of `kind` conversations through
/// `reached` (plan E3.3, D23): a later read reached the head they marked,
/// so the entries they stood for are in the history now. Returns how many
/// were removed.
///
/// # Errors
/// `reached` is not a stored seq (above `i64::MAX`), or the write failed.
pub fn close(
    conn: &Connection,
    owner_key: &str,
    kind: rekindle_types::message::ConversationType,
    writer: &str,
    reached: u64,
) -> rusqlite::Result<usize> {
    let reached =
        i64::try_from(reached).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "DELETE FROM messages WHERE owner_key = ?1 AND conversation_type = ?2 \
         AND sender_key = ?3 AND history_gap_through IS NOT NULL \
         AND history_gap_through <= ?4",
        params![owner_key, kind.as_str(), writer, reached],
    )
}

/// Whether `writer`'s stored messages of `conversation` hold every outbox
/// stream seq strictly between `after` (the stream seq at our cursor) and
/// `next` (the first entry's after a retire mark), plan E2.6 G4: a
/// retired stretch whose every entry came as a hint is no gap of ours.
/// Only stored rows count, never the stream seqs alone (a hidden real gap
/// is worse than a false one): a `next` at or below `after` holds nothing.
///
/// # Errors
/// A seq does not fit a row, or the query failed.
pub fn stretch_held(
    conn: &Connection,
    owner_key: &str,
    conversation: (&str, rekindle_types::message::ConversationType),
    writer: &str,
    (after, next): (u64, u64),
) -> rusqlite::Result<bool> {
    let Some(between) = next.checked_sub(after).and_then(|d| d.checked_sub(1)) else {
        return Ok(false);
    };
    let to_row =
        |v: u64| i64::try_from(v).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()));
    let held: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT stream_seq) FROM messages WHERE owner_key = ?1 \
         AND conversation_id = ?2 AND conversation_type = ?3 AND sender_key = ?4 \
         AND history_gap_through IS NULL AND stream_seq > ?5 AND stream_seq < ?6",
        params![
            owner_key,
            conversation.0,
            conversation.1.as_str(),
            writer,
            to_row(after)?,
            to_row(next)?
        ],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(held).is_ok_and(|h| h == between))
}
