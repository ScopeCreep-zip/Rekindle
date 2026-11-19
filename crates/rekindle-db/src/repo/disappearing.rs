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

/// Set the timer on a conversation; whether a row was there to set.
///
/// # Errors
/// The write failed.
pub fn set_timer(
    conn: &Connection,
    owner_key: &str,
    conversation: &str,
    kind: ConversationType,
    seconds: u32,
) -> rusqlite::Result<bool> {
    let sql = match kind {
        ConversationType::GroupDm => {
            "UPDATE group_dms SET disappearing_seconds = ?3 WHERE owner_key = ?1 AND group_id = ?2"
        }
        _ => {
            "UPDATE friends SET disappearing_seconds = ?3 WHERE owner_key = ?1 AND public_key = ?2"
        }
    };
    Ok(conn.execute(sql, params![owner_key, conversation, seconds])? > 0)
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
