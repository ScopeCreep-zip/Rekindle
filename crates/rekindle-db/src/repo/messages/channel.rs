//! Channel messages in `messages` (`conversation_type = 'channel'`), on both
//! hosts (plan E3.1 E2: the desktop's `message_repo` moved here so the
//! daemon stores channel messages too).

use rekindle_types::message::SendState;
use rusqlite::{params, Connection};

use crate::row::get;

/// Full column set for a channel-message row insert. Mirrors the `messages`
/// table shape; grouping the fields keeps the insert under the argument
/// budget while each field stays explicit at the construction site.
pub struct ChannelMessageInsert<'a> {
    pub owner_key: &'a str,
    pub channel_id: &'a str,
    pub sender_key: &'a str,
    pub body: &'a str,
    pub timestamp: i64,
    pub is_read: bool,
    pub mek_generation: Option<i64>,
    pub message_id: &'a str,
    pub lamport_ts: u64,
    pub automod_blurred: bool,
    pub forwarded_from_author: Option<&'a str>,
    pub flags: u32,
    pub attachment_json: Option<&'a str>,
    /// `Delivered` for a received row; our own post's state otherwise
    /// (plan D1).
    pub send_state: SendState,
    /// The message id of the message this one replies to; stored as its
    /// local row (`reply_to_id`) when we hold that parent.
    pub reply_to: Option<&'a str>,
    /// A received message that mentions us (plan E3.1, D25), from
    /// `rekindle_channel::mentions`; never set on our own post.
    pub mentions_me: bool,
}

/// Insert one channel message row.
///
/// # Errors
/// The insert failed.
pub fn insert(conn: &Connection, msg: &ChannelMessageInsert<'_>) -> rusqlite::Result<()> {
    let lamport_ts = i64::try_from(msg.lamport_ts).unwrap_or(i64::MAX);
    let reply_to_id = match msg.reply_to {
        Some(parent) => row_id_by_message_id(conn, msg.owner_key, msg.channel_id, parent)?,
        None => None,
    };
    conn.execute(
        "INSERT INTO messages \
         (owner_key, conversation_id, conversation_type, sender_key, body, automod_blurred, timestamp, is_read, \
          mek_generation, message_id, lamport_ts, forwarded_from_author, flags, attachment_json, \
          send_state, reply_to_id, mentions_me) \
         VALUES (?, ?, 'channel', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            msg.owner_key,
            msg.channel_id,
            msg.sender_key,
            msg.body,
            i32::from(msg.automod_blurred),
            msg.timestamp,
            msg.is_read,
            msg.mek_generation,
            msg.message_id,
            lamport_ts,
            msg.forwarded_from_author,
            i64::from(msg.flags),
            msg.attachment_json,
            msg.send_state.as_str(),
            reply_to_id,
            i32::from(msg.mentions_me),
        ],
    )?;
    Ok(())
}

/// The local row id of the channel message `message_id`, when we hold it.
///
/// # Errors
/// The query failed.
pub fn row_id_by_message_id(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT id FROM messages \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND message_id = ?3 LIMIT 1",
        params![owner_key, channel_id, message_id],
        |row| row.get(0),
    )
    .optional()
}

/// Whether the gate hid our stored channel message `message_id`
/// (`automod_blurred`), when we hold it.
///
/// # Errors
/// The query failed.
pub fn held_blurred(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<Option<bool>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT automod_blurred FROM messages \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND message_id = ?3 LIMIT 1",
        params![owner_key, channel_id, message_id],
        |row| row.get::<_, i64>(0).map(|blurred| blurred != 0),
    )
    .optional()
}

/// The local row (`reply_to_id`) that our stored channel message
/// `message_id` replies to, or `None` when it is not a reply or not stored.
pub fn reply_row_id(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension as _;
    let row: Option<Option<i64>> = conn
        .query_row(
            "SELECT reply_to_id FROM messages \
             WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
               AND message_id = ?3 LIMIT 1",
            params![owner_key, channel_id, message_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(row.flatten())
}

/// The sender and plaintext of one stored channel message, the fields a
/// forward re-encrypts (the forwarder never fetches the source from the DHT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardSource {
    pub sender_key: String,
    pub body: String,
    /// The message id of the stored parent it replies to.
    pub reply_to: Option<String>,
}

/// Look up a stored channel message by its protocol message id.
///
/// # Errors
/// The query failed.
pub fn find_by_message_id(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<Option<ForwardSource>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT m.sender_key, m.body, p.message_id FROM messages m \
         LEFT JOIN messages p ON p.id = m.reply_to_id \
         WHERE m.owner_key = ?1 AND m.conversation_id = ?2 AND m.conversation_type = 'channel' \
           AND m.message_id = ?3 LIMIT 1",
        params![owner_key, channel_id, message_id],
        |row| {
            Ok(ForwardSource {
                sender_key: row.get(0)?,
                body: row.get(1)?,
                reply_to: row.get(2)?,
            })
        },
    )
    .optional()
}

/// One stored channel message, read strictly (plan D2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChannelMessage {
    pub id: i64,
    pub sender_key: String,
    pub body: String,
    pub automod_blurred: bool,
    pub timestamp: i64,
    pub message_id: Option<String>,
    pub forwarded_from_author: Option<String>,
    pub attachment_json: Option<String>,
    pub flags: u32,
    pub send_state: SendState,
}

/// Up to `limit` messages of `channel_id`, oldest first; with `before`, only
/// those older than it. Ordered by the message lamport (the wall clock when
/// a row has none), ties by sender, as every peer orders a channel.
///
/// # Errors
/// The query failed or a row is malformed.
pub fn page(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    before: Option<i64>,
    limit: u32,
) -> rusqlite::Result<Vec<StoredChannelMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, sender_key, body, automod_blurred, timestamp, message_id, forwarded_from_author, \
                attachment_json, flags, send_state \
         FROM messages \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND (?3 IS NULL OR timestamp < ?3) \
         ORDER BY COALESCE(NULLIF(lamport_ts, 0), timestamp) DESC, sender_key DESC LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![owner_key, channel_id, before, limit], |row| {
        let stored: String = get(row, "send_state")?;
        let send_state = SendState::parse(&stored).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown send state {stored}").into(),
            )
        })?;
        Ok(StoredChannelMessage {
            id: get(row, "id")?,
            sender_key: get(row, "sender_key")?,
            body: get(row, "body")?,
            automod_blurred: get::<i64>(row, "automod_blurred")? != 0,
            timestamp: get(row, "timestamp")?,
            message_id: get(row, "message_id")?,
            forwarded_from_author: get(row, "forwarded_from_author")?,
            attachment_json: get(row, "attachment_json")?,
            flags: get(row, "flags")?,
            send_state,
        })
    })?;
    let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    out.reverse();
    Ok(out)
}

/// Delete one stored channel message by its protocol id; whether a row went.
///
/// # Errors
/// The delete failed.
pub fn delete(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'channel' AND message_id = ?3",
        params![owner_key, channel_id, message_id],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn row(id: &str, ts: i64, lamport: u64) -> ChannelMessageInsert<'_> {
        ChannelMessageInsert {
            owner_key: OWNER,
            channel_id: "ch",
            sender_key: "alice",
            body: id,
            timestamp: ts,
            is_read: false,
            mek_generation: Some(1),
            message_id: id,
            lamport_ts: lamport,
            automod_blurred: false,
            forwarded_from_author: None,
            flags: 0,
            attachment_json: None,
            send_state: SendState::Delivered,
            mentions_me: false,
            reply_to: None,
        }
    }

    #[test]
    fn page_is_oldest_first_and_before_bounds_it() {
        let conn = fixture::conn();
        fixture::community(&conn, "c");
        for (id, ts, l) in [("a", 10, 1), ("b", 20, 2), ("c", 30, 3)] {
            insert(&conn, &row(id, ts, l)).unwrap();
        }
        let all: Vec<_> = page(&conn, OWNER, "ch", None, 10).unwrap();
        let ids: Vec<_> = all.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        let older = page(&conn, OWNER, "ch", Some(30), 10).unwrap();
        assert_eq!(older.len(), 2);
        assert_eq!(
            find_by_message_id(&conn, OWNER, "ch", "b")
                .unwrap()
                .unwrap()
                .body,
            "b"
        );
        assert!(delete(&conn, OWNER, "ch", "b").unwrap());
        assert!(find_by_message_id(&conn, OWNER, "ch", "b")
            .unwrap()
            .is_none());
    }
}
