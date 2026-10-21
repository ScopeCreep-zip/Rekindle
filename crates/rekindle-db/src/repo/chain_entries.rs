//! `chain_entries` (plan E2.5, D23/D24): the entries of our DM chain to
//! each peer, by chain seq, with the `dm` outbox row each one carried and
//! when it was appended; read to decide how far the chain may retire.

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rusqlite::{params, Connection};

/// How long an acknowledged entry stays on our chain, the peer's inbox,
/// before it may be retired: [`DELIVERY_HORIZON`] (D24, ruling Q2b).
pub const CHAIN_RETENTION: Duration = DELIVERY_HORIZON;

fn sql_seq(seq: u64) -> rusqlite::Result<i64> {
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
}
