//! Disappearing messages (plan E7.1): each conversation's timer
//! (`friends.disappearing_seconds`, `group_dms.disappearing_seconds`), the
//! start of a received message's timer at its first display, and the
//! expiry task's reads and deletes over `messages.expire_at`.

use rusqlite::{params, Connection, OptionalExtension as _};

use rekindle_types::message::ConversationType;

/// The timer on a conversation: the peer's friend row (`Dm`) or the
/// group's row (`GroupDm`); 0 when off or not held.
///
/// # Errors
/// The read failed.
pub fn timer(
    conn: &Connection,
    owner_key: &str,
    conversation: &str,
    kind: ConversationType,
) -> rusqlite::Result<u32> {
    let sql = match kind {
        ConversationType::GroupDm => {
            "SELECT disappearing_seconds FROM group_dms WHERE owner_key = ?1 AND group_id = ?2"
        }
        _ => "SELECT disappearing_seconds FROM friends WHERE owner_key = ?1 AND public_key = ?2",
    };
    Ok(conn
        .query_row(sql, params![owner_key, conversation], |row| row.get(0))
        .optional()?
        .unwrap_or(0))
}

/// A timer change to apply ([`change_timer`]): `set_by` (`owner_key` for
/// our own change) set the timer on
/// `conversation` to `seconds` at `now` (ms); `notice_id` is the client id
/// minted for the notice row.
#[derive(Debug, Clone, Copy)]
pub struct TimerChange<'a> {
    pub owner_key: &'a str,
    pub conversation: &'a str,
    pub kind: ConversationType,
    pub seconds: u32,
    pub set_by: &'a str,
    pub now: i64,
    pub notice_id: &'a str,
}

/// What [`change_timer`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerChanged {
    /// No friend or group row holds the conversation; nothing written.
    NotHeld,
    /// The timer already was that value; nothing written (Signal ignores
    /// an unchanged `expireTimer`, `DataMessageProcessor`).
    Unchanged,
    /// The timer changed and its notice row was stored.
    Changed,
}

/// Set the timer on a conversation and, when it changed, store the
/// timeline notice (`messages.timer_notice_seconds`) in the same
/// transaction. The notice carries the timer it set, as Signal's
/// `EXPIRATION_UPDATE` row does: ours expires at `now` + timer, a received
/// one from its first display ([`start_on_display`]); a notice turning the
/// timer off stays.
///
/// # Errors
/// The transaction failed; nothing was written.
pub fn change_timer(conn: &Connection, c: &TimerChange<'_>) -> rusqlite::Result<TimerChanged> {
    let tx = conn.unchecked_transaction()?;
    let (read, write) = match c.kind {
        ConversationType::GroupDm => (
            "SELECT disappearing_seconds FROM group_dms WHERE owner_key = ?1 AND group_id = ?2",
            "UPDATE group_dms SET disappearing_seconds = ?3 WHERE owner_key = ?1 AND group_id = ?2",
        ),
        _ => (
            "SELECT disappearing_seconds FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            "UPDATE friends SET disappearing_seconds = ?3 WHERE owner_key = ?1 AND public_key = ?2",
        ),
    };
    let owner = c.owner_key;
    let held: Option<u32> = tx
        .query_row(read, params![owner, c.conversation], |row| row.get(0))
        .optional()?;
    let outcome = match held {
        None => TimerChanged::NotHeld,
        Some(s) if s == c.seconds => TimerChanged::Unchanged,
        Some(_) => {
            tx.execute(write, params![owner, c.conversation, c.seconds])?;
            let ours = c.set_by == owner;
            let expire_at = (ours && c.seconds > 0).then(|| c.now + i64::from(c.seconds) * 1000);
            tx.execute(
                "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, \
                 body, timestamp, is_read, send_state, message_id, expire_seconds, expire_at, \
                 timer_notice_seconds) \
                 VALUES (?1, ?2, ?3, ?4, '', ?5, 1, 'delivered', ?6, ?7, ?8, ?7)",
                params![
                    owner,
                    c.conversation,
                    c.kind.as_str(),
                    c.set_by,
                    c.now,
                    c.notice_id,
                    c.seconds,
                    expire_at
                ],
            )?;
            TimerChanged::Changed
        }
    };
    tx.commit()?;
    Ok(outcome)
}

/// Start the timers of the received messages in `conversation` that have
/// one and have not started: their first display is `now` (ms). How many
/// started.
///
/// # Errors
/// The write failed.
pub fn start_on_display(
    conn: &Connection,
    owner_key: &str,
    conversation: &str,
    now: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE messages SET expire_at = ?3 + expire_seconds * 1000 \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND sender_key <> ?1 \
         AND expire_seconds > 0 AND expire_at IS NULL",
        params![owner_key, conversation, now],
    )
}

/// The earliest `expire_at` held, ms; `None` when no timer runs.
///
/// # Errors
/// The read failed.
pub fn next_expiry(conn: &Connection, owner_key: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT MIN(expire_at) FROM messages WHERE owner_key = ?1 AND expire_at IS NOT NULL",
        params![owner_key],
        |row| row.get(0),
    )
}

/// The messages of one conversation that expired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expired {
    pub conversation_id: String,
    pub conversation_type: ConversationType,
    /// Their client ids (a row with none is deleted and not named).
    pub client_ids: Vec<String>,
}

/// Delete, in one transaction, every message whose `expire_at` is at or
/// before `now` (ms), with their `outbox`, `message_send_log` and
/// `unprocessed` rows; the FTS triggers drop their index rows. What went, by conversation.
///
/// # Errors
/// The transaction failed; nothing was deleted.
pub fn delete_expired(
    conn: &Connection,
    owner_key: &str,
    now: i64,
) -> rusqlite::Result<Vec<Expired>> {
    let tx = conn.unchecked_transaction()?;
    let rows: Vec<(String, String, Option<String>)> = {
        let mut stmt = tx.prepare(
            "SELECT conversation_id, conversation_type, message_id FROM messages \
             WHERE owner_key = ?1 AND expire_at IS NOT NULL AND expire_at <= ?2 \
             ORDER BY conversation_id, id",
        )?;
        let rows = stmt
            .query_map(params![owner_key, now], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    let mut out: Vec<Expired> = Vec::new();
    for (conversation_id, kind, client_id) in rows {
        let conversation_type = kind.parse::<ConversationType>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
        })?;
        if let Some(id) = &client_id {
            // Its sends stop, its logged plaintext goes (no RetryRequest can
            // resend it), and a received one's leftover row cannot replay it.
            for sql in [
                "DELETE FROM outbox WHERE owner_key = ?1 AND client_id = ?2",
                "DELETE FROM message_send_log WHERE owner_key = ?1 AND client_id = ?2",
                "DELETE FROM unprocessed WHERE owner_key = ?1 AND client_id = ?2",
            ] {
                tx.execute(sql, params![owner_key, id])?;
            }
        }
        match out.last_mut() {
            Some(e) if e.conversation_id == conversation_id => e.client_ids.extend(client_id),
            _ => out.push(Expired {
                conversation_id,
                conversation_type,
                client_ids: client_id.into_iter().collect(),
            }),
        }
    }
    tx.execute(
        "DELETE FROM messages WHERE owner_key = ?1 AND expire_at IS NOT NULL AND expire_at <= ?2",
        params![owner_key, now],
    )?;
    tx.commit()?;
    Ok(out)
}

#[cfg(test)]
#[path = "disappearing_tests.rs"]
mod tests;
