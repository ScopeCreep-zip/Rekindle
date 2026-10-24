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
/// next after `head` (the chain's newest landed seq) and every seq still
/// pending (one append in flight per stream). An assignment whose row left
/// the outbox before its append landed is released first, so the seqs
/// stay contiguous behind the head. The entry has no `appended_at` until
/// [`landed`].
///
/// # Errors
/// A seq does not fit a row, or the write failed; nothing was assigned.
pub fn assign(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    stream_seq: i64,
    head: u64,
) -> rusqlite::Result<u64> {
    let tx = conn.unchecked_transaction()?;
    if let Some(seq) = assigned(&tx, owner_key, peer, stream_seq)? {
        return Ok(seq);
    }
    tx.execute(
        "DELETE FROM chain_entries WHERE owner_key = ?1 AND peer = ?2 \
         AND appended_at IS NULL AND NOT EXISTS (SELECT 1 FROM outbox o \
         WHERE o.owner_key = ?1 AND o.stream = 'dm:' || ?2 \
         AND o.stream_seq = chain_entries.stream_seq)",
        params![owner_key, peer],
    )?;
    let pending: Option<i64> = tx.query_row(
        "SELECT MAX(chain_seq) FROM chain_entries WHERE owner_key = ?1 AND peer = ?2",
        params![owner_key, peer],
        |row| row.get(0),
    )?;
    let pending = pending.and_then(|s| u64::try_from(s).ok()).unwrap_or(0);
    let seq = head.max(pending) + 1;
    tx.execute(
        "INSERT INTO chain_entries (owner_key, peer, chain_seq, stream_seq, appended_at) \
         VALUES (?1, ?2, ?3, ?4, NULL)",
        params![owner_key, peer, sql_seq(seq)?, stream_seq],
    )?;
    tx.commit()?;
    Ok(seq)
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
        "UPDATE chain_entries SET appended_at = ?5 WHERE owner_key = ?1 AND peer = ?2 \
         AND chain_seq BETWEEN ?3 AND ?4 AND appended_at IS NULL",
        params![owner_key, peer, sql_seq(first)?, sql_seq(last)?, now],
    )
}

/// The highest chain seq our chain to `peer` may retire through: every
/// entry up to it was appended before `cutoff` (`now - DELIVERY_HORIZON`)
/// and no entry up to it carries a `dm` row still in the outbox (the peer
/// acknowledged each, or it settled at its deadline). `None`: nothing.
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
             AND o.stream_seq = w.stream_seq \
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
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn outbox_row(conn: &Connection, stream_seq: i64) {
        conn.execute(
            "INSERT INTO outbox (owner_key, stream, stream_seq, client_id, kind, payload, \
             state, next_at, deadline) VALUES (?1, 'dm:bob', ?2, ?3, 'dm', x'00', 'sent', 0, 0)",
            params![OWNER, stream_seq, format!("c{stream_seq}")],
        )
        .unwrap();
    }

    /// Only acknowledged entries older than the cutoff retire, and an
    /// unacknowledged one holds back every entry after it.
    #[test]
    fn the_retire_point_stops_at_the_horizon_and_the_first_unacknowledged_entry() {
        let conn = fixture::conn();
        for (chain, stream, at) in [(1, 1, 10), (2, 2, 20), (3, 3, 30), (4, 4, 40), (5, 5, 90)] {
            record(&conn, OWNER, "bob", chain, stream, at).unwrap();
        }
        assert_eq!(retirable(&conn, OWNER, "bob", 50).unwrap(), Some(4));
        assert_eq!(retirable(&conn, OWNER, "bob", 25).unwrap(), Some(2));
        outbox_row(&conn, 3);
        assert_eq!(retirable(&conn, OWNER, "bob", 50).unwrap(), Some(2));
        assert_eq!(retirable(&conn, OWNER, "bob", 5).unwrap(), None);
        assert_eq!(forget_through(&conn, OWNER, "bob", 2).unwrap(), 2);
        assert_eq!(retirable(&conn, OWNER, "bob", 50).unwrap(), None);
        assert_eq!(retirable(&conn, OWNER, "eve", 50).unwrap(), None);
    }

    /// A group chain retires by age alone, to the newest entry older than
    /// the cutoff.
    #[test]
    fn a_group_chain_retires_by_age_only() {
        let conn = fixture::conn();
        let stream = "group_dm:00";
        for (chain, stream_seq, at) in [(1, 1, 10), (2, 2, 20), (3, 3, 90)] {
            record(&conn, OWNER, stream, chain, stream_seq, at).unwrap();
        }
        assert_eq!(retirable_by_age(&conn, OWNER, stream, 50).unwrap(), Some(2));
        assert_eq!(retirable_by_age(&conn, OWNER, stream, 5).unwrap(), None);
        assert_eq!(forget_through(&conn, OWNER, stream, 2).unwrap(), 2);
        assert_eq!(retirable_by_age(&conn, OWNER, stream, 50).unwrap(), None);
    }
}

#[cfg(test)]
mod assign_tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn queued(conn: &Connection, stream_seq: i64) {
        conn.execute(
            "INSERT INTO outbox (owner_key, stream, stream_seq, client_id, kind, payload, \
             state, next_at, deadline) VALUES (?1, 'dm:bob', ?2, ?3, 'dm', x'00', 'queued', 0, 0)",
            params![OWNER, stream_seq, format!("c{stream_seq}")],
        )
        .unwrap();
    }

    /// Seqs follow the head and every pending one, a re-assignment (a
    /// re-hint, a pass after a crash) reuses the recorded seq, and an
    /// unlanded entry is never retired.
    #[test]
    fn a_row_keeps_its_chain_seq_and_the_next_follows_the_pending() {
        let conn = fixture::conn();
        for s in [4, 5, 6] {
            queued(&conn, s);
        }
        assert_eq!(assign(&conn, OWNER, "bob", 4, 7).unwrap(), 8);
        assert_eq!(assign(&conn, OWNER, "bob", 5, 7).unwrap(), 9);
        assert_eq!(assign(&conn, OWNER, "bob", 4, 9).unwrap(), 8);
        assert_eq!(assigned(&conn, OWNER, "bob", 5).unwrap(), Some(9));
        assert_eq!(retirable(&conn, OWNER, "bob", i64::MAX).unwrap(), None);
        assert_eq!(landed(&conn, OWNER, "bob", (8, 8), 10).unwrap(), 1);
        // Row 5 left before its append landed: its seq is released.
        conn.execute("DELETE FROM outbox WHERE stream_seq = 5", [])
            .unwrap();
        assert_eq!(assign(&conn, OWNER, "bob", 6, 8).unwrap(), 9);
    }
}

/// Our chain to `peer` has a hole below an assigned seq (an assignment
/// released after its row left before landing): every seq not landed is
/// released, so the next pass assigns them again from the head. How many.
///
/// # Errors
/// The write failed.
pub fn release_unlanded(conn: &Connection, owner_key: &str, peer: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM chain_entries WHERE owner_key = ?1 AND peer = ?2 AND appended_at IS NULL",
        params![owner_key, peer],
    )
}
