//! Channel posts waiting for their channel's key (plan E3.2 S7l, F9-1): a
//! post composed before we hold the key is a `channel_post` row in state
//! `awaiting_key`, its echo the post's `messages` row (whose body is the
//! plaintext, in this SQLCipher database), until the key is installed and
//! the row is released to its write (`queued`), or it fails at its
//! deadline, or our departure discards it.

use rekindle_types::message::SendState;
use rusqlite::{params, Connection};

use super::{insert_row, next_seq, project, NewRow};

/// Store `row` at the end of its stream, `awaiting_key`, with our channel
/// post `message_id`, already written `awaiting_key`, as its echo. Returns
/// the row's `stream_seq`.
///
/// # Errors
/// The write failed, or no waiting post is `message_id`; nothing is stored.
pub fn enqueue_waiting_for_message(
    conn: &Connection,
    row: &NewRow<'_>,
    message_id: &str,
) -> rusqlite::Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let echo_id: i64 = tx.query_row(
        "SELECT id FROM messages WHERE owner_key = ?1 AND message_id = ?2 \
         AND conversation_type = 'channel' AND send_state = 'awaiting_key'",
        params![row.owner_key, message_id],
        |r| r.get(0),
    )?;
    let seq = next_seq(&tx, row)?;
    insert_row(&tx, row, seq, Some(echo_id))?;
    tx.execute(
        "UPDATE outbox SET state = 'awaiting_key' WHERE owner_key = ?1 AND client_id = ?2",
        params![row.owner_key, row.client_id],
    )?;
    tx.commit()?;
    Ok(seq)
}

/// Release the waiting post `client_id` to its stream: `queued`, due at
/// `now`, with `deadline` (its write's), its echo with it. Returns whether it was
/// waiting; a row another release took first, or none, is left alone, so
/// a post is written once.
///
/// # Errors
/// The write failed; nothing changed.
pub fn release_waiting(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    (now, deadline): (i64, i64),
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let released = tx.execute(
        "UPDATE outbox SET state = 'queued', next_at = ?3, deadline = ?4, wedge_reason = NULL \
         WHERE owner_key = ?1 AND client_id = ?2 AND state = 'awaiting_key'",
        params![owner_key, client_id, now, deadline],
    )? > 0;
    if released {
        project(&tx, owner_key, client_id, SendState::Queued)?;
    }
    tx.commit()?;
    Ok(released)
}

/// Park the post `client_id` (the user's Retry of one whose key still has
/// not come) waiting for the key again until `deadline`, its echo with
/// it. Returns whether a `queued` or `failed` row was parked.
///
/// # Errors
/// The write failed; nothing changed.
pub fn wait_for_key(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    deadline: i64,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let parked = tx.execute(
        "UPDATE outbox SET state = 'awaiting_key', attempts = 0, next_at = ?3, deadline = ?3, \
         wedge_reason = NULL WHERE owner_key = ?1 AND client_id = ?2 \
         AND kind = 'channel_post' AND state IN ('queued', 'failed')",
        params![owner_key, client_id, deadline],
    )? > 0;
    if parked {
        project(&tx, owner_key, client_id, SendState::AwaitingKey)?;
    }
    tx.commit()?;
    Ok(parked)
}
