//! `outbox`, `outbox_streams` and `received`: the storage of
//! `rekindle_outbox` (plan E2.5, D22). The engine owns the policy (state
//! merge, backoff, deadlines); these functions only store what it decides.
//! A row update writes the local echo's `messages.send_state` in the same
//! transaction, so the projection never disagrees with its row.

use rekindle_types::message::SendState;
use rusqlite::{params, Connection, OptionalExtension as _};

use super::messages::{insert_dm, NewDm};

/// A send intent to store.
#[derive(Debug, Clone, Copy)]
pub struct NewRow<'a> {
    pub owner_key: &'a str,
    pub stream: &'a str,
    pub client_id: &'a str,
    /// The stream kind's stored form (`rekindle_outbox::StreamKind`).
    pub kind: &'a str,
    pub payload: &'a [u8],
    pub next_at: i64,
    pub deadline: i64,
}

/// One stored send intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub stream: String,
    pub stream_seq: i64,
    pub client_id: String,
    pub kind: String,
    pub payload: Vec<u8>,
    pub state: SendState,
    pub attempts: u32,
    pub next_at: i64,
    pub deadline: i64,
    pub wedge_reason: Option<String>,
}

/// A row's new scheduling and state, as the engine settled one attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update<'a> {
    pub state: SendState,
    pub attempts: u32,
    pub next_at: i64,
    pub deadline: i64,
    pub wedge_reason: Option<&'a str>,
}

/// Store `row` at the end of its stream, with its local echo (a direct
/// message, written `queued` under the row's client id) in the same
/// transaction. Returns the row's `stream_seq`.
///
/// # Errors
/// The write failed; nothing is stored.
pub fn enqueue_with_echo(
    conn: &Connection,
    row: &NewRow<'_>,
    echo: Option<&NewDm<'_>>,
) -> rusqlite::Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let echo_id = match echo {
        Some(dm) => {
            let dm = NewDm {
                owner_key: row.owner_key,
                message_id: row.client_id,
                send_state: SendState::Queued,
                ..*dm
            };
            Some(insert_dm(&tx, &dm)?.id)
        }
        None => None,
    };
    let seq: i64 = tx.query_row(
        "INSERT INTO outbox_streams (owner_key, stream, last_seq) VALUES (?1, ?2, 1) \
         ON CONFLICT(owner_key, stream) DO UPDATE SET last_seq = last_seq + 1 \
         RETURNING last_seq",
        params![row.owner_key, row.stream],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO outbox (owner_key, stream, stream_seq, client_id, kind, payload, state, \
         attempts, next_at, deadline, wedge_reason, echo_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', 0, ?7, ?8, NULL, ?9)",
        params![
            row.owner_key,
            row.stream,
            seq,
            row.client_id,
            row.kind,
            row.payload,
            row.next_at,
            row.deadline,
            echo_id
        ],
    )?;
    tx.commit()?;
    Ok(seq)
}

const COLS: &str = "o.stream, o.stream_seq, o.client_id, o.kind, o.payload, o.state, o.attempts, \
                    o.next_at, o.deadline, o.wedge_reason";

/// A row whose stream has no earlier row still to send (`queued`) or
/// wedged (`failed`): the stream head and the rows already past it.
const UNBLOCKED: &str = "NOT EXISTS (SELECT 1 FROM outbox h WHERE h.owner_key = o.owner_key \
                         AND h.stream = o.stream AND h.stream_seq < o.stream_seq \
                         AND h.state IN ('queued', 'failed'))";

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let stored: String = row.get(5)?;
    let state = SendState::parse(&stored).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            format!("unknown send state {stored}").into(),
        )
    })?;
    Ok(Row {
        stream: row.get(0)?,
        stream_seq: row.get(1)?,
        client_id: row.get(2)?,
        kind: row.get(3)?,
        payload: row.get(4)?,
        state,
        attempts: row.get(6)?,
        next_at: row.get(7)?,
        deadline: row.get(8)?,
        wedge_reason: row.get(9)?,
    })
}

fn query_rows(
    conn: &Connection,
    filter: &str,
    args: impl rusqlite::Params,
) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM outbox o WHERE {filter}"))?;
    let rows = stmt.query_map(args, read_row)?.collect();
    rows
}

/// The rows to attempt at `now`: unsettled (`queued`, or `sent` and
/// waiting for the peer's ack), due, and not behind their stream's head.
/// Oldest due first; within a stream in `stream_seq` order.
///
/// # Errors
/// The query failed.
pub fn due(conn: &Connection, owner_key: &str, now: i64) -> rusqlite::Result<Vec<Row>> {
    query_rows(
        conn,
        &format!(
            "o.owner_key = ?1 AND o.state IN ('queued', 'sent') AND o.next_at <= ?2 \
             AND {UNBLOCKED} ORDER BY o.next_at, o.stream, o.stream_seq"
        ),
        params![owner_key, now],
    )
}

/// The unsettled rows whose deadline is at or before `now`.
///
/// # Errors
/// The query failed.
pub fn expired(conn: &Connection, owner_key: &str, now: i64) -> rusqlite::Result<Vec<Row>> {
    query_rows(
        conn,
        "o.owner_key = ?1 AND o.state IN ('queued', 'sent') AND o.deadline <= ?2 \
         ORDER BY o.stream, o.stream_seq",
        params![owner_key, now],
    )
}

/// When the engine next has work: the earliest `next_at` of an unblocked
/// unsettled row, or the earliest deadline of any unsettled row. `None`
/// when nothing is unsettled.
///
/// # Errors
/// The query failed.
pub fn next_wake(conn: &Connection, owner_key: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        &format!(
            "SELECT MIN(t) FROM ( \
               SELECT o.next_at AS t FROM outbox o WHERE o.owner_key = ?1 \
                 AND o.state IN ('queued', 'sent') AND {UNBLOCKED} \
               UNION ALL \
               SELECT o.deadline FROM outbox o WHERE o.owner_key = ?1 \
                 AND o.state IN ('queued', 'sent'))"
        ),
        params![owner_key],
        |r| r.get(0),
    )
}

/// The row whose client id is `client_id`.
///
/// # Errors
/// The query failed.
pub fn get(conn: &Connection, owner_key: &str, client_id: &str) -> rusqlite::Result<Option<Row>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM outbox o WHERE o.owner_key = ?1 AND o.client_id = ?2"),
        params![owner_key, client_id],
        read_row,
    )
    .optional()
}

/// The row at `stream_seq` of `stream` (a peer acknowledges by it).
///
/// # Errors
/// The query failed.
pub fn get_by_seq(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    stream_seq: i64,
) -> rusqlite::Result<Option<Row>> {
    conn.query_row(
        &format!(
            "SELECT {COLS} FROM outbox o WHERE o.owner_key = ?1 AND o.stream = ?2 \
             AND o.stream_seq = ?3"
        ),
        params![owner_key, stream, stream_seq],
        read_row,
    )
    .optional()
}

fn project(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    state: SendState,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET send_state = ?3 WHERE owner_key = ?1 AND id = \
         (SELECT echo_id FROM outbox WHERE owner_key = ?1 AND client_id = ?2)",
        params![owner_key, client_id, state.as_str()],
    )?;
    Ok(())
}

/// Store the engine's decision for `client_id`, and project its state onto
/// the local echo. Returns whether the row exists.
///
/// # Errors
/// The write failed; nothing changed.
pub fn update(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    change: &Update<'_>,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let found = tx.execute(
        "UPDATE outbox SET state = ?3, attempts = ?4, next_at = ?5, deadline = ?6, \
         wedge_reason = ?7 WHERE owner_key = ?1 AND client_id = ?2",
        params![
            owner_key,
            client_id,
            change.state.as_str(),
            change.attempts,
            change.next_at,
            change.deadline,
            change.wedge_reason
        ],
    )? > 0;
    if found {
        project(&tx, owner_key, client_id, change.state)?;
    }
    tx.commit()?;
    Ok(found)
}

/// Settle `client_id` at `state`: project it onto the local echo and
/// delete the row (the stream head moves past it). Returns whether the
/// row existed.
///
/// # Errors
/// The write failed; nothing changed.
pub fn settle(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    state: SendState,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    project(&tx, owner_key, client_id, state)?;
    let found = tx.execute(
        "DELETE FROM outbox WHERE owner_key = ?1 AND client_id = ?2",
        params![owner_key, client_id],
    )? > 0;
    tx.commit()?;
    Ok(found)
}

/// Record that `client_id` from `stream` was applied. `true` the first
/// time; `false` for a resend already applied.
///
/// # Errors
/// The write failed.
pub fn mark_received(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    client_id: &str,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "INSERT INTO received (owner_key, stream, client_id) VALUES (?1, ?2, ?3) \
         ON CONFLICT DO NOTHING",
        params![owner_key, stream, client_id],
    )? > 0)
}

/// How many send intents are not settled (D3's `PendingWork`): every row
/// still in the outbox, a wedged `failed` one included.
///
/// # Errors
/// The query failed.
pub fn unsettled_count(conn: &Connection, owner_key: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM outbox WHERE owner_key = ?1",
        params![owner_key],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests;
