//! `chain_entries` (plan E2.5, D23/D24): the entries of our DM chain to
//! each peer, and of our group chain in each group (plan E2.6, keyed by
//! the stream `group_dm:<group id>`), by chain seq, with the outbox row
//! each one carried and when it was appended; read to decide how far the
//! chain may retire.

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rusqlite::{params, Connection, OptionalExtension as _};

/// How long an acknowledged entry stays on our chain, the peer's inbox,
/// before it may be retired: [`DELIVERY_HORIZON`] (D24, ruling Q2b).
pub const CHAIN_RETENTION: Duration = DELIVERY_HORIZON;

/// A seq as a SQLite integer; one above `i64::MAX` is refused.
pub(crate) fn sql_seq(seq: u64) -> rusqlite::Result<i64> {
    i64::try_from(seq).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
}

/// Our chain to `peer` holds entry `chain_seq`, the frame of our `dm` row
/// `stream_seq`, appended at `now`.
///
/// # Errors
/// A seq does not fit a row, or the write failed.
pub fn record(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    chain_seq: u64,
    stream_seq: i64,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO chain_entries (owner_key, peer, chain_seq, stream_seq, appended_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
        params![owner_key, peer, sql_seq(chain_seq)?, stream_seq, now],
    )?;
    Ok(())
}

/// The chain seq assigned to our `dm` row `stream_seq` to `peer` (live A/B
/// finding 71 refined): a re-hint, or a pass after a crash between the
/// assignment and the append, reuses it.
///
/// # Errors
/// The query failed.
pub fn assigned(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    stream_seq: i64,
) -> rusqlite::Result<Option<u64>> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT chain_seq FROM chain_entries \
             WHERE owner_key = ?1 AND peer = ?2 AND stream_seq = ?3",
            params![owner_key, peer, stream_seq],
            |row| row.get(0),
        )
        .optional()?;
    Ok(seq.and_then(|s| u64::try_from(s).ok()))
}

/// Assign our `dm` row `stream_seq` to `peer` its chain seq, the one its
/// hint carries (live A/B finding 71 refined): the one it holds, or the
/// next after the conversation's chain head (`friends.dm_chain_head`, the
/// highest seq assigned, kept locally so no DHT read precedes a hint),
/// which advances to it. The entry has no `appended_at` until [`landed`],
/// and no frame until [`set_frame`]. `None`: no friend row to number.
///
/// # Errors
/// A seq does not fit a row, or the write failed; nothing was assigned.
pub fn assign(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    stream_seq: i64,
) -> rusqlite::Result<Option<u64>> {
    let tx = conn.unchecked_transaction()?;
    if let Some(seq) = assigned(&tx, owner_key, peer, stream_seq)? {
        return Ok(Some(seq));
    }
    let head: Option<i64> = tx
        .query_row(
            "UPDATE friends SET dm_chain_head = dm_chain_head + 1 \
             WHERE owner_key = ?1 AND public_key = ?2 RETURNING dm_chain_head",
            params![owner_key, peer],
            |row| row.get(0),
        )
        .optional()?;
    let Some(seq) = head.and_then(|s| u64::try_from(s).ok()) else {
        return Ok(None);
    };
    tx.execute(
        "INSERT INTO chain_entries (owner_key, peer, chain_seq, stream_seq, appended_at) \
         VALUES (?1, ?2, ?3, ?4, NULL)",
        params![owner_key, peer, sql_seq(seq)?, stream_seq],
    )?;
    tx.commit()?;
    Ok(Some(seq))
}

/// Seal entry `chain_seq` with `frame` (its first hint), kept until it
/// lands so the append completes whatever happens to the row (a deadline,
/// a wedge, a restart): a seq sealed into a hint is never released. The
/// seal is set once (review LV22 H1): a later framing of the same entry
/// never replaces the bytes an append may already carry, so every append
/// of the seq writes the same entry. Returns the sealed frame, `None`
/// when the entry is gone or landed.
///
/// # Errors
/// A seq does not fit a row, or the write failed.
pub fn set_frame(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    chain_seq: u64,
    frame: &[u8],
) -> rusqlite::Result<Option<Vec<u8>>> {
    let seq = sql_seq(chain_seq)?;
    conn.execute(
        "UPDATE chain_entries SET frame = ?4 WHERE owner_key = ?1 AND peer = ?2 \
         AND chain_seq = ?3 AND appended_at IS NULL AND frame IS NULL",
        params![owner_key, peer, seq, frame],
    )?;
    conn.query_row(
        "SELECT frame FROM chain_entries WHERE owner_key = ?1 AND peer = ?2 \
         AND chain_seq = ?3 AND appended_at IS NULL",
        params![owner_key, peer, seq],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Entry `chain_seq`, just assigned, was never sealed into a frame (its
/// framing failed): it is the head, and the head steps back. A sealed one
/// is never released.
///
/// # Errors
/// A seq does not fit a row, or the write failed.
pub fn unassign(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    chain_seq: u64,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let gone = tx.execute(
        "DELETE FROM chain_entries WHERE owner_key = ?1 AND peer = ?2 AND chain_seq = ?3 \
         AND frame IS NULL AND appended_at IS NULL",
        params![owner_key, peer, sql_seq(chain_seq)?],
    )? > 0;
    if gone {
        tx.execute(
            "UPDATE friends SET dm_chain_head = ?3 - 1 \
             WHERE owner_key = ?1 AND public_key = ?2 AND dm_chain_head = ?3",
            params![owner_key, peer, sql_seq(chain_seq)?],
        )?;
    }
    tx.commit()?;
    Ok(gone)
}

/// The sealed entries of our chain to `peer` still to land, in chain seq
/// order: `(chain_seq, stream_seq, frame)`.
///
/// # Errors
/// The query failed.
pub fn pending(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
) -> rusqlite::Result<Vec<(u64, i64, Vec<u8>)>> {
    let mut stmt = conn.prepare(
        "SELECT chain_seq, stream_seq, frame FROM chain_entries WHERE owner_key = ?1 \
         AND peer = ?2 AND appended_at IS NULL AND frame IS NOT NULL ORDER BY chain_seq",
    )?;
    let rows = stmt.query_map(params![owner_key, peer], |row| {
        let seq: i64 = row.get(0)?;
        Ok((u64::try_from(seq).unwrap_or(0), row.get(1)?, row.get(2)?))
    })?;
    rows.collect()
}

/// The entries `first..=last` of our chain to `peer` landed at `now`.
///
/// # Errors
/// A seq does not fit a row, or the write failed.
pub fn landed(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    (first, last): (u64, u64),
    now: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE chain_entries SET appended_at = ?5, frame = NULL WHERE owner_key = ?1 AND peer = ?2 \
         AND chain_seq BETWEEN ?3 AND ?4 AND appended_at IS NULL",
        params![owner_key, peer, sql_seq(first)?, sql_seq(last)?, now],
    )
}

/// The highest chain seq our chain to `peer` may retire through: every
/// entry up to it was appended before `cutoff` (`now - DELIVERY_HORIZON`)
/// and no entry up to it carries a `dm` row still to settle in the outbox
/// (the peer acknowledged each, or it failed at its deadline: a failed row
/// stays for the user's resend, whose hint carries the entry itself, and
/// does not hold the chain past `DELIVERY_HORIZON`). `None`: nothing.
///
/// # Errors
/// The query failed.
pub fn retirable(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    cutoff: i64,
) -> rusqlite::Result<Option<u64>> {
    let seq: Option<i64> = conn.query_row(
        "SELECT MAX(c.chain_seq) FROM chain_entries c \
         WHERE c.owner_key = ?1 AND c.peer = ?2 AND c.appended_at < ?3 \
         AND c.chain_seq < COALESCE((\
             SELECT MIN(w.chain_seq) FROM chain_entries w JOIN outbox o \
             ON o.owner_key = w.owner_key AND o.stream = 'dm:' || w.peer \
             AND o.stream_seq = w.stream_seq AND o.state != 'failed' \
             WHERE w.owner_key = ?1 AND w.peer = ?2), 9223372036854775807)",
        params![owner_key, peer, cutoff],
        |row| row.get(0),
    )?;
    Ok(seq.and_then(|s| u64::try_from(s).ok()))
}

/// The highest chain seq our group chain `stream` (`group_dm:<group id>`)
/// may retire through: the newest entry appended before `cutoff`
/// (`now - CHAIN_RETENTION`). A group has no acknowledgements (plan E2.6:
/// a row settles `sent` when its entry lands), so the rule is age only,
/// and a member still behind the mark is shown a history gap. `None`:
/// nothing that old.
///
/// # Errors
/// The query failed.
pub fn retirable_by_age(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    cutoff: i64,
) -> rusqlite::Result<Option<u64>> {
    let seq: Option<i64> = conn.query_row(
        "SELECT MAX(chain_seq) FROM chain_entries \
         WHERE owner_key = ?1 AND peer = ?2 AND appended_at < ?3",
        params![owner_key, stream, cutoff],
        |row| row.get(0),
    )?;
    Ok(seq.and_then(|s| u64::try_from(s).ok()))
}

/// The peers our chain holds noted entries for: the conversations the
/// friend maintenance's retire tick looks at.
///
/// # Errors
/// The query failed.
pub fn peers(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT peer FROM chain_entries WHERE owner_key = ?1 ORDER BY peer")?;
    let rows = stmt
        .query_map(params![owner_key], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

/// The chain retired through `chain_seq`: forget those entries.
///
/// # Errors
/// The write failed.
pub fn forget_through(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    chain_seq: u64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM chain_entries WHERE owner_key = ?1 AND peer = ?2 AND chain_seq <= ?3",
        params![owner_key, peer, sql_seq(chain_seq)?],
    )
}

#[cfg(test)]
#[path = "chain_entries_tests.rs"]
mod tests;

/// Whether our `dm` row `stream_seq` to `peer` is on the chain (its entry
/// landed).
///
/// # Errors
/// The query failed.
pub fn has_landed(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    stream_seq: i64,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM chain_entries WHERE owner_key = ?1 AND peer = ?2 \
         AND stream_seq = ?3 AND appended_at IS NOT NULL)",
        params![owner_key, peer, stream_seq],
        |row| row.get(0),
    )
}

#[cfg(test)]
#[path = "chain_entries_assign_tests.rs"]
mod assign_tests;
