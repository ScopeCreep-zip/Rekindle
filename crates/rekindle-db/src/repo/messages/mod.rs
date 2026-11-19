//! `messages`: direct messages (plan E2.1) and the delivery state of our
//! own sent messages (plan D1, C7.14), which the outbox's rows project
//! (`repo::outbox`, plan E2.5); channel messages are [`channel`] (E3.1 E2).

pub mod attachments;
pub mod channel;
pub mod channel_changes;

use rekindle_types::message::SendState;
use rusqlite::{params, Connection};

/// A message's delivery state, if it exists.
///
/// # Errors
/// The query failed or the stored state is not in the vocabulary.
pub fn send_state(
    conn: &Connection,
    owner_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<SendState>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT send_state FROM messages WHERE owner_key = ?1 AND message_id = ?2",
        params![owner_key, message_id],
        |row| {
            let stored: String = row.get(0)?;
            SendState::parse(&stored).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    format!("unknown send state {stored}").into(),
                )
            })
        },
    )
    .optional()
}

/// A direct message, as a chat window shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmRow {
    pub id: i64,
    /// The client id its author minted (plan E2.3).
    pub message_id: Option<String>,
    pub peer_key: String,
    pub sender_key: String,
    pub body: String,
    pub timestamp: i64,
    pub is_read: bool,
    pub send_state: SendState,
    /// A history-gap marker's last retired seq (plan E2.5, D23); `None`
    /// for a message.
    pub history_gap_through: Option<u64>,
    /// Its disappearing timer and deadline (plan E7.1).
    pub expire_seconds: u32,
    pub expire_at: Option<i64>,
    /// A timer-change notice's new timer (plan E7.1); `None` for a message.
    pub timer_notice_seconds: Option<u32>,
}

/// A direct message to store (plan E2.3): `message_id` is its client id,
/// minted by its author, and `reply_to` the parent's.
#[derive(Debug, Clone, Copy)]
pub struct NewDm<'a> {
    pub owner_key: &'a str,
    /// The conversation: the peer, or a group's id (32 hex, plan E2.6).
    pub peer_key: &'a str,
    /// `Dm`, or `GroupDm` for a group's message.
    pub conversation_type: rekindle_types::message::ConversationType,
    pub sender_key: &'a str,
    pub body: &'a str,
    pub timestamp: i64,
    /// Ours is read; a received one is unread until the window marks it.
    pub is_read: bool,
    pub message_id: &'a str,
    /// Resolved to the parent's row; `NULL` when the parent is not held.
    pub reply_to: Option<&'a str>,
    /// Ours is `queued` until the send's outcome (`sent` or `failed`)
    /// replaces it; a received one is `delivered` (plan audit D-3).
    pub send_state: SendState,
    /// Its place in its DM stream (ours: our outbox seq; received: the
    /// sender's), which a `ReadUpTo` names (plan E2.5).
    pub stream_seq: Option<i64>,
    /// The timer it carried (plan E7.1; 0 = none) and, for our own copy,
    /// when it is deleted (sent + timer); a received one's starts at its
    /// first display (`repo::disappearing::start_on_display`).
    pub expire_seconds: u32,
    pub expire_at: Option<i64>,
}

/// A stored direct message: its row, and whether this call stored it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredDm {
    pub id: i64,
    /// `false` when its client id was already held (a replay of an
    /// applied message, D24): nothing was written.
    pub inserted: bool,
}

/// Store a direct message. A message whose client id is already held from
/// the same sender is not stored again: the held row is returned, not
/// inserted. Messages are told apart by `(sender, client id)`, never by
/// time, so two in one millisecond are both stored, and a peer reusing
/// our client id stores a row of its own. A reply points at the first
/// held row with the parent's client id.
///
/// # Errors
/// The write failed.
pub fn insert_dm(conn: &Connection, dm: &NewDm<'_>) -> rusqlite::Result<StoredDm> {
    let inserted = conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, is_read, message_id, reply_to_id, send_state, stream_seq, expire_seconds, \
         expire_at) \
         VALUES (?1, ?2, ?11, ?3, ?4, ?5, ?6, ?7, \
                 (SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
                  AND message_id = ?8 ORDER BY id LIMIT 1), ?9, ?10, ?12, ?13) \
         ON CONFLICT(owner_key, conversation_id, sender_key, message_id) \
         WHERE message_id IS NOT NULL DO NOTHING",
        params![
            dm.owner_key,
            dm.peer_key,
            dm.sender_key,
            dm.body,
            dm.timestamp,
            dm.is_read,
            dm.message_id,
            dm.reply_to,
            dm.send_state.as_str(),
            dm.stream_seq,
            dm.conversation_type.as_str(),
            dm.expire_seconds,
            dm.expire_at
        ],
    )? > 0;
    let id = conn.query_row(
        "SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND sender_key = ?3 AND message_id = ?4",
        params![dm.owner_key, dm.peer_key, dm.sender_key, dm.message_id],
        |row| row.get(0),
    )?;
    Ok(StoredDm { id, inserted })
}

pub(crate) fn dm_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DmRow> {
    let stored: String = row.get(6)?;
    Ok(DmRow {
        id: row.get(0)?,
        peer_key: row.get(1)?,
        sender_key: row.get(2)?,
        body: row.get(3)?,
        timestamp: row.get(4)?,
        is_read: row.get(5)?,
        message_id: row.get(7)?,
        send_state: SendState::parse(&stored).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                format!("unknown send state {stored}").into(),
            )
        })?,
        history_gap_through: row
            .get::<_, Option<i64>>(8)?
            .map(|seq| {
                u64::try_from(seq).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        8,
                        rusqlite::types::Type::Integer,
                        Box::new(e),
                    )
                })
            })
            .transpose()?,
        expire_seconds: row.get(9)?,
        expire_at: row.get(10)?,
        timer_notice_seconds: row.get(11)?,
    })
}

pub(crate) const DM_COLS: &str =
    "id, conversation_id, sender_key, body, timestamp, is_read, send_state, \
                       message_id, history_gap_through, expire_seconds, expire_at, timer_notice_seconds";

/// The order of a conversation's history, newest first: by the time each
/// message was written (the sender's, which its frames carry), then by
/// sender key and the sender's stream seq, so one sender's messages keep
/// their stream order whatever order they arrived in, and both sides list
/// the conversation alike (E2.B F3, D23).
pub(crate) const HISTORY_ORDER: &str = "timestamp DESC, sender_key DESC, stream_seq DESC, id DESC";

/// The newest `limit` messages with `peer_key` before `before` (a
/// timestamp, ms; all when `None`), oldest first, in [`HISTORY_ORDER`].
///
/// # Errors
/// The query failed.
pub fn dm_history(
    conn: &Connection,
    owner_key: &str,
    peer_key: &str,
    limit: u32,
    before: Option<i64>,
) -> rusqlite::Result<Vec<DmRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {DM_COLS} FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'dm' AND timestamp < ?3 ORDER BY {HISTORY_ORDER} LIMIT ?4"
    ))?;
    let mut rows = stmt
        .query_map(
            params![owner_key, peer_key, before.unwrap_or(i64::MAX), limit],
            dm_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.reverse();
    Ok(rows)
}

/// One of our own direct messages, by its client id, with its parent's
/// client id when it is a reply: what a resend frames again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnDm {
    pub row: DmRow,
    pub reply_to: Option<String>,
}

/// Our direct message whose client id is `message_id`; `None` when we sent
/// none with that id.
///
/// # Errors
/// The query failed.
pub fn own_dm(
    conn: &Connection,
    owner_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<OwnDm>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT m.id, m.conversation_id, m.sender_key, m.body, m.timestamp, m.is_read, \
         m.send_state, m.message_id, m.history_gap_through, m.expire_seconds, m.expire_at, \
         m.timer_notice_seconds, p.message_id FROM messages m \
         LEFT JOIN messages p ON p.id = m.reply_to_id \
         WHERE m.owner_key = ?1 AND m.sender_key = ?1 AND m.message_id = ?2 \
         AND m.conversation_type = 'dm'",
        params![owner_key, message_id],
        |row| {
            Ok(OwnDm {
                row: dm_row(row)?,
                reply_to: row.get(12)?,
            })
        },
    )
    .optional()
}

/// The stream seq of `peer_key`'s message `message_id` (its client id),
/// as it arrived; `None` when no such received message is held or it
/// carried none (plan E2.5: the window reports the newest one it rendered).
///
/// # Errors
/// The read failed.
pub fn received_stream_seq(
    conn: &Connection,
    owner_key: &str,
    peer_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT stream_seq FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'dm' AND sender_key = ?2 AND message_id = ?3",
        params![owner_key, peer_key, message_id],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Mark every message from `peer_key` read.
///
/// # Errors
/// The write failed.
pub fn mark_dm_read(conn: &Connection, owner_key: &str, peer_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET is_read = 1 WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'dm' AND is_read = 0",
        params![owner_key, peer_key],
    )?;
    Ok(())
}

/// The latest message of each direct conversation, newest first, at most
/// `limit`.
///
/// # Errors
/// The query failed.
pub fn dm_inbox(conn: &Connection, owner_key: &str, limit: u32) -> rusqlite::Result<Vec<DmRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {DM_COLS} FROM messages m WHERE owner_key = ?1 AND conversation_type = 'dm' \
         AND id = (SELECT id FROM messages n WHERE n.owner_key = m.owner_key \
                   AND n.conversation_id = m.conversation_id AND n.conversation_type = 'dm' \
                   AND n.history_gap_through IS NULL AND n.timer_notice_seconds IS NULL \
                   ORDER BY n.timestamp DESC, n.sender_key DESC, n.stream_seq DESC, n.id DESC \
                   LIMIT 1) \
         ORDER BY timestamp DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![owner_key, limit], dm_row)?.collect();
    rows
}

/// The conversation a message of ours or a peer's belongs to, named by its
/// client id: `(conversation id, type)` of the first row holding it.
///
/// # Errors
/// The query failed or the stored type is not in the vocabulary.
pub fn conversation_of(
    conn: &Connection,
    owner_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<(String, rekindle_types::message::ConversationType)>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT conversation_id, conversation_type FROM messages \
         WHERE owner_key = ?1 AND message_id = ?2 ORDER BY id LIMIT 1",
        params![owner_key, message_id],
        |row| {
            Ok((
                crate::row::get(row, "conversation_id")?,
                crate::row::get_enum(row, "conversation_type")?,
            ))
        },
    )
    .optional()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
