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
    /// When the message was written: its local echo's timestamp, which
    /// every attempt's frame carries as its send time, so the peer orders
    /// it by when it was written, not by the attempt that reached them
    /// (E2.B F3); `None` for a send with no echo.
    pub sent_at: Option<i64>,
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
    let seq = append(&tx, row, echo)?;
    tx.commit()?;
    Ok(seq)
}

/// Store `rows`, in order, each at the end of its stream with no echo, in
/// one transaction: all or none (an invite's `InviteCreated` and its
/// revoke-unless-landed row, review RV4 M3). Returns each row's
/// `stream_seq`.
///
/// # Errors
/// A write failed; nothing is stored.
pub fn enqueue_all(conn: &Connection, rows: &[NewRow<'_>]) -> rusqlite::Result<Vec<i64>> {
    let tx = conn.unchecked_transaction()?;
    let seqs = rows
        .iter()
        .map(|row| append(&tx, row, None))
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    tx.commit()?;
    Ok(seqs)
}

/// Store `row` at the end of its stream, with its local echo if any (written
/// `queued` under the row's client id and seq), inside the caller's
/// transaction (a friend's removal stores its `Unfriended` row with the
/// state, plan audit D-1). Returns the row's `stream_seq`.
///
/// # Errors
/// The write failed.
pub fn append(
    conn: &Connection,
    row: &NewRow<'_>,
    echo: Option<&NewDm<'_>>,
) -> rusqlite::Result<i64> {
    let seq = next_seq(conn, row)?;
    let echo_id = match echo {
        Some(dm) => {
            let dm = NewDm {
                owner_key: row.owner_key,
                message_id: row.client_id,
                send_state: SendState::Queued,
                stream_seq: Some(seq),
                timestamp: written_at(conn, row.owner_key, dm)?,
                ..*dm
            };
            Some(insert_dm(conn, &dm)?.id)
        }
        None => None,
    };
    insert_row(conn, row, seq, echo_id)?;
    Ok(seq)
}

/// The time our echo `dm` is written at: its own, or our newest message in
/// its conversation's when the clock went back, so our messages' times
/// never run against their stream order (the history orders by time,
/// E2.B F3).
fn written_at(conn: &Connection, owner_key: &str, dm: &NewDm<'_>) -> rusqlite::Result<i64> {
    let newest: Option<i64> = conn.query_row(
        "SELECT MAX(timestamp) FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = ?3 AND sender_key = ?1",
        params![owner_key, dm.peer_key, dm.conversation_type.as_str()],
        |r| r.get(0),
    )?;
    Ok(newest.map_or(dm.timestamp, |t| t.max(dm.timestamp)))
}

/// Store `row` at the end of its stream with our channel post
/// `message_id`, already written `queued`, as its echo (a post's
/// `messages.send_state` only this row moves from then on). Returns the
/// row's `stream_seq`.
///
/// # Errors
/// The write failed, or no queued post is `message_id`; nothing is stored.
pub fn enqueue_for_message(
    conn: &Connection,
    row: &NewRow<'_>,
    message_id: &str,
) -> rusqlite::Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let echo_id: i64 = tx.query_row(
        "SELECT id FROM messages WHERE owner_key = ?1 AND message_id = ?2 \
         AND conversation_type = 'channel' AND send_state = 'queued'",
        params![row.owner_key, message_id],
        |r| r.get(0),
    )?;
    let seq = next_seq(&tx, row)?;
    insert_row(&tx, row, seq, Some(echo_id))?;
    tx.commit()?;
    Ok(seq)
}

fn next_seq(conn: &Connection, row: &NewRow<'_>) -> rusqlite::Result<i64> {
    conn.query_row(
        "INSERT INTO outbox_streams (owner_key, stream, last_seq) VALUES (?1, ?2, 1) \
         ON CONFLICT(owner_key, stream) DO UPDATE SET last_seq = last_seq + 1 \
         RETURNING last_seq",
        params![row.owner_key, row.stream],
        |r| r.get(0),
    )
}

fn insert_row(
    conn: &Connection,
    row: &NewRow<'_>,
    seq: i64,
    echo_id: Option<i64>,
) -> rusqlite::Result<()> {
    conn.execute(
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
    Ok(())
}

/// The rows of the streams scoped to one peer (`dm:<peer>`,
/// `friend_handshake:<peer>`; the scope follows the kind's `:`), as a
/// condition on `o` whose peer is the next placeholder's value.
macro_rules! to_peer {
    ($peer:literal) => {
        concat!(
            "o.kind IN ('dm', 'friend_handshake') \
             AND substr(o.stream, instr(o.stream, ':') + 1) = ",
            $peer
        )
    };
}

/// The condition [`to_peer!`] for the friend `f` with a send not settled
/// (`queued`, or `sent` and waiting for the peer).
pub(crate) const UNSETTLED_TO_FRIEND: &str = concat!(
    "EXISTS (SELECT 1 FROM outbox o WHERE o.owner_key = f.owner_key AND ",
    to_peer!("f.public_key"),
    " AND o.state IN ('queued', 'sent'))"
);

/// Delete every row of the streams to `peer` (the friendship ended, or
/// they were blocked or forgotten). Returns how many went.
///
/// # Errors
/// The write failed.
pub fn delete_to_peer(conn: &Connection, owner_key: &str, peer: &str) -> rusqlite::Result<usize> {
    conn.execute(
        concat!(
            "DELETE FROM outbox AS o WHERE o.owner_key = ?1 AND ",
            to_peer!("?2")
        ),
        params![owner_key, peer],
    )
}

/// Project `state` onto `client_id`'s echo, never lowering it (D22: the
/// echo's state is monotone, merged with max). `queued` < `sent` <
/// `delivered` < `read`; `failed` ranks with `sent`, so it replaces an
/// unacked echo but never a `delivered` or `read` one, and a `failed` echo
/// takes any state (a retry queues it again, and its failure reason goes).
/// A re-handed-off in-flight row
/// is `queued` again in the outbox, but its delivered echo stays delivered.
fn project(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    state: SendState,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET send_state = ?3, \
         failure_reason = CASE ?3 WHEN 'failed' THEN failure_reason ELSE NULL END \
         WHERE owner_key = ?1 AND id = \
         (SELECT echo_id FROM outbox WHERE owner_key = ?1 AND client_id = ?2) \
         AND (CASE ?3 WHEN 'awaiting_key' THEN 0 WHEN 'queued' THEN 0 WHEN 'sent' THEN 1 \
         WHEN 'failed' THEN 1 \
         WHEN 'delivered' THEN 2 ELSE 3 END) >= \
         (CASE send_state WHEN 'queued' THEN 0 WHEN 'sent' THEN 1 \
         WHEN 'delivered' THEN 2 WHEN 'read' THEN 3 ELSE -1 END)",
        params![owner_key, client_id, state.as_str()],
    )?;
    Ok(())
}

/// Bring every landed, unacked (`sent`) row of `stream` due at `at` when
/// that is earlier than its `next_at`; how many moved. A later `next_at`
/// is never set, so a sooner re-ask is kept.
///
/// # Errors
/// The write failed; nothing changed.
pub fn reschedule_sent(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    at: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE outbox SET next_at = ?3 WHERE owner_key = ?1 AND stream = ?2 \
         AND state = 'sent' AND next_at > ?3",
        params![owner_key, stream, at],
    )
}

/// A row [`update`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Updated {
    /// The peer's ack recorded before the row's send landed.
    pub peer_ack: Option<SendState>,
}

/// Store the engine's decision for `client_id`, and project its state onto
/// the local echo. Returns `None` when no row is `client_id`, else the
/// peer's ack [`note_ack`] recorded on it (`delivered` or `read`), read in
/// the same statement, so a landing never misses an ack stored before it.
///
/// # Errors
/// The write failed; nothing changed.
pub fn update(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    change: &Update<'_>,
) -> rusqlite::Result<Option<Updated>> {
    let tx = conn.unchecked_transaction()?;
    let acked: Option<Option<String>> = tx
        .query_row(
            "UPDATE outbox SET state = ?3, attempts = ?4, next_at = ?5, deadline = ?6, \
             wedge_reason = ?7 WHERE owner_key = ?1 AND client_id = ?2 RETURNING peer_ack",
            params![
                owner_key,
                client_id,
                change.state.as_str(),
                change.attempts,
                change.next_at,
                change.deadline,
                change.wedge_reason
            ],
            |r| r.get(0),
        )
        .optional()?;
    if acked.is_some() {
        project(&tx, owner_key, client_id, change.state)?;
    }
    tx.commit()?;
    acked
        .map(|ack| {
            Ok(Updated {
                peer_ack: ack.map(|a| parse_state(&a, 0)).transpose()?,
            })
        })
        .transpose()
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

mod reads;
use reads::parse_state;
pub use reads::{
    due, expired, get, get_by_seq, next_wake, prefix_rows, stream_rows, waiting_for_key,
};

mod waiting;
pub use waiting::{enqueue_waiting_for_message, release_waiting, wait_for_key};

mod ending;
mod receipts;
pub use ending::{
    fail_to_peer, queue_friend_list, supersede, Ended, ListWrite, FRIEND_LIST_STREAM,
};
pub use receipts::{
    deliver_chain_through, deliver_seq, land, note_ack, read_through, reschedule_queued, Landing,
};

#[cfg(test)]
mod tests;

/// Store a row of `stream` (`<kind>:<scope>`) for a test.
#[cfg(test)]
pub(crate) fn queue_for_test(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    client_id: &str,
    payload: &[u8],
) -> i64 {
    let kind = stream.split(':').next().unwrap();
    let row = NewRow {
        owner_key,
        stream,
        client_id,
        kind,
        payload,
        next_at: 0,
        deadline: i64::MAX,
    };
    enqueue_with_echo(conn, &row, None).unwrap()
}
