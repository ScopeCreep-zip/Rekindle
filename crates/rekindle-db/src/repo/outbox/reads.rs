//! Reading `outbox` rows: what is due, a stream's rows, one row, and when
//! the engine next has work (plan E2.5, D22).

use rekindle_types::message::SendState;
use rusqlite::{params, Connection, OptionalExtension as _};

use super::Row;

const COLS: &str = "o.stream, o.stream_seq, o.client_id, o.kind, o.payload, o.state, o.attempts, \
                    o.next_at, o.deadline, o.wedge_reason, \
                    (SELECT e.timestamp FROM messages e WHERE e.id = o.echo_id), o.depends_on";

/// Whether the failed row `h` still holds its stream: any failed row
/// does, except on a `dm` stream (D22 corrected 2026-10-23: a DM is never
/// held behind an earlier one's DHT write). There the chain order is the
/// seqs: a sealed entry lands ahead of the later rows whatever happens to
/// its row (`chain_entries::pending`), and an unsealed one took no seq, so
/// a deadline failure, kept for the user's resend, no longer stops every
/// later DM to the friend (live A/B finding 73, review LV22 hypothesis B).
macro_rules! holds_chain {
    () => {
        "h.stream NOT LIKE 'dm:%'"
    };
}

/// A row of a stream whose rows wait only for what they name (plan HOL1,
/// lead decision 47): a peer's `friend_handshake` stream, whose frames need
/// no order among themselves (the receiver gates each on what it really
/// needs: the friendship, the session), so one unreceipted frame never
/// holds the unrelated ones behind it. Which kinds those are is the
/// caller's (`rekindle_outbox::StreamKind::ordered`, lead decision 51: one
/// definition drives both the store and the engine's wake), bound as the
/// parameter `$kinds`, a [`kinds_param`] list.
macro_rules! by_dependency {
    ($kinds:literal) => {
        concat!(
            "instr(",
            $kinds,
            ", ',' || substr(o.stream, 1, instr(o.stream, ':') - 1) || ',') > 0"
        )
    };
}

/// The row `o` names as its dependency has settled (left the table), or it
/// names none. A `failed` dependency still holds it: the engine fails its
/// dependents with it, and the user's Retry queues them again together.
macro_rules! dependency_settled {
    () => {
        "(o.depends_on IS NULL OR NOT EXISTS (SELECT 1 FROM outbox d \
         WHERE d.owner_key = o.owner_key AND d.client_id = o.depends_on))"
    };
}

/// A row due at `?2` that nothing holds: on a dependency stream
/// ([`by_dependency`]) its named dependency settled; on any other stream no
/// earlier row is wedged (`failed`) or still to send (`queued`) and not due
/// at `?2`: the stream's run of due rows from its head, which the engine
/// attempts together in `stream_seq` order and stops at the first that
/// stays behind (so a batch, the `dm` chain's page, carries them at once,
/// plan E2.B F2), and the rows already past it.
const DUE_UNBLOCKED: &str = concat!(
    "((",
    by_dependency!("?3"),
    " AND ",
    dependency_settled!(),
    ") OR (NOT ",
    by_dependency!("?3"),
    " AND NOT EXISTS (SELECT 1 FROM outbox h WHERE h.owner_key = o.owner_key \
     AND h.stream = o.stream AND h.stream_seq < o.stream_seq \
     AND ((h.state = 'failed' AND ",
    holds_chain!(),
    ") OR (h.state = 'queued' AND h.next_at > ?2)))))"
);

/// A row nothing holds whatever its time: on a dependency stream its named
/// dependency settled; on any other no earlier row still to send
/// (`queued`) or wedged (`failed`): the stream head and the rows already
/// past it, whose `next_at` is when the engine next has work.
const UNBLOCKED: &str = concat!(
    "((",
    by_dependency!("?2"),
    " AND ",
    dependency_settled!(),
    ") OR (NOT ",
    by_dependency!("?2"),
    " AND NOT EXISTS (SELECT 1 FROM outbox h WHERE h.owner_key = o.owner_key \
     AND h.stream = o.stream AND h.stream_seq < o.stream_seq \
     AND (h.state = 'queued' OR (h.state = 'failed' AND ",
    holds_chain!(),
    ")))))"
);

/// The stream kinds `by_dependency` (the stored forms, `dm`,
/// `friend_handshake`) as the query binds them: `,a,b,`, or empty for none.
fn kinds_param(by_dependency: &[&str]) -> String {
    if by_dependency.is_empty() {
        return String::new();
    }
    format!(",{},", by_dependency.join(","))
}

pub(super) fn parse_state(stored: &str, column: usize) -> rusqlite::Result<SendState> {
    SendState::parse(stored).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            format!("unknown send state {stored}").into(),
        )
    })
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let stored: String = row.get(5)?;
    let state = parse_state(&stored, 5)?;
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
        sent_at: row.get(10)?,
        depends_on: row.get(11)?,
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
/// Oldest due first; within a stream in `stream_seq` order. The streams of
/// the kinds `by_dependency` wait only for their named row.
///
/// # Errors
/// The query failed.
pub fn due(
    conn: &Connection,
    owner_key: &str,
    now: i64,
    by_dependency: &[&str],
) -> rusqlite::Result<Vec<Row>> {
    query_rows(
        conn,
        &format!(
            "o.owner_key = ?1 AND o.state IN ('queued', 'sent') AND o.next_at <= ?2 \
             AND {DUE_UNBLOCKED} ORDER BY o.next_at, o.stream, o.stream_seq"
        ),
        params![owner_key, now, kinds_param(by_dependency)],
    )
}

/// Every row still on a stream whose name starts with `prefix` (a call
/// session's `media_key` streams, one per recipient; a peer's
/// `friend_handshake` stream), in stream and `stream_seq` order, whatever
/// its state: a row is in the table until it settles, so `queued`, `sent`
/// and a wedged `failed` one awaiting Retry are all unsettled, and the
/// session's end or a cancelled request discards them together (live A/B
/// finding 70 refined: a request wedged on its first send stayed).
///
/// # Errors
/// The query failed.
pub fn prefix_rows(conn: &Connection, owner_key: &str, prefix: &str) -> rusqlite::Result<Vec<Row>> {
    query_rows(
        conn,
        "o.owner_key = ?1 AND substr(o.stream, 1, length(?2)) = ?2 \
         ORDER BY o.stream, o.stream_seq",
        params![owner_key, prefix],
    )
}

/// The unsettled rows (`queued`, or `sent` and waiting) of `stream`, in
/// `stream_seq` order: a stream whose send is a write the record pool
/// reports settled (`channel_post`) settles them together.
///
/// # Errors
/// The query failed.
pub fn stream_rows(conn: &Connection, owner_key: &str, stream: &str) -> rusqlite::Result<Vec<Row>> {
    query_rows(
        conn,
        "o.owner_key = ?1 AND o.stream = ?2 AND o.state IN ('queued', 'sent') \
         ORDER BY o.stream_seq",
        params![owner_key, stream],
    )
}

/// The unsettled rows whose deadline is at or before `now`.
///
/// # Errors
/// The query failed.
pub fn expired(conn: &Connection, owner_key: &str, now: i64) -> rusqlite::Result<Vec<Row>> {
    query_rows(
        conn,
        "o.owner_key = ?1 AND o.state IN ('awaiting_key', 'queued', 'sent') AND o.deadline <= ?2 \
         ORDER BY o.stream, o.stream_seq",
        params![owner_key, now],
    )
}

/// When the engine next has work: the earliest `next_at` of an unblocked
/// unsettled row, or the earliest deadline of any unsettled row. `None`
/// when nothing is unsettled. A row behind a `queued` one counts as
/// blocked: whether that one's append is in flight, which a pass walks
/// past, only the engine knows, and it adds those streams' rows itself
/// (`rekindle_outbox` `Outbox::busy_wake`, E2.5i). Counting them here would
/// wake a pass for a row a deferred head still holds back, over and over.
///
/// # Errors
/// The query failed.
pub fn next_wake(
    conn: &Connection,
    owner_key: &str,
    by_dependency: &[&str],
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        &format!(
            "SELECT MIN(t) FROM ( \
               SELECT o.next_at AS t FROM outbox o WHERE o.owner_key = ?1 \
                 AND o.state IN ('queued', 'sent') AND {UNBLOCKED} \
               UNION ALL \
               SELECT o.deadline FROM outbox o WHERE o.owner_key = ?1 \
                 AND o.state IN ('awaiting_key', 'queued', 'sent'))"
        ),
        params![owner_key, kinds_param(by_dependency)],
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

/// The channel posts waiting for their channel's key (`awaiting_key`, plan
/// E3.2 S7l, F9-1), in stream and `stream_seq` order: each channel slot's
/// posts first-composed first.
///
/// # Errors
/// The query failed.
pub fn waiting_for_key(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<Row>> {
    query_rows(
        conn,
        "o.owner_key = ?1 AND o.kind = 'channel_post' AND o.state = 'awaiting_key' \
         ORDER BY o.stream, o.stream_seq",
        params![owner_key],
    )
}

/// The newest row of `stream` still in the table whose client id starts
/// with one of `prefixes`: the row a new one waits for (plan HOL1, decision
/// 47: a friend-gated frame waits for our unsettled first contact). `None`
/// when there is none, and the new row waits for nothing.
///
/// # Errors
/// The query failed.
pub fn latest_with_prefix(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    prefixes: &[String],
) -> rusqlite::Result<Option<String>> {
    if prefixes.is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT client_id FROM outbox WHERE owner_key = ?1 AND stream = ?2 \
         ORDER BY stream_seq DESC",
    )?;
    let ids = stmt.query_map(params![owner_key, stream], |r| r.get::<_, String>(0))?;
    for id in ids {
        let id = id?;
        if prefixes.iter().any(|p| id.starts_with(p.as_str())) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// `client_id`'s stored state and the peer's ack noted on it while it was
/// `queued` ([`super::note_ack`]); `None` when no row is `client_id`. The
/// outbox engine reads it in the same call as its write, so a state is
/// merged against the row as it is now, never as an attempt began.
///
/// # Errors
/// The query failed.
pub fn stored_state(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
) -> rusqlite::Result<Option<(SendState, Option<SendState>)>> {
    conn.query_row(
        "SELECT state, peer_ack FROM outbox WHERE owner_key = ?1 AND client_id = ?2",
        params![owner_key, client_id],
        |r| {
            let state: String = r.get(0)?;
            let ack: Option<String> = r.get(1)?;
            Ok((
                parse_state(&state, 0)?,
                ack.map(|a| parse_state(&a, 1)).transpose()?,
            ))
        },
    )
    .optional()
}
