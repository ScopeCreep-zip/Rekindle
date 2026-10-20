//! `unprocessed`: decrypted 1:1 messages not yet applied (plan E2.3, D24).
//!
//! The receive order is decrypt → commit the ratchet and the plaintext in
//! one transaction (`crate::ratchet`) → ack → apply, and the apply deletes
//! the row ([`delete`]). A crash anywhere after the commit leaves the row,
//! and startup replays it ([`take_for_replay`]); the ratchet already
//! consumed the message key, so the replay is the only copy. This is
//! Signal Desktop's `unprocessed` table (`ts/sql/Server.node.ts`
//! `saveUnprocessed`, `getAllUnprocessedIds`,
//! `getUnprocessedByIdsAndIncrementAttempts`, `removeUnprocessed`).

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rusqlite::{params, Connection};

use super::identity_text;
use crate::row;

/// How old a row may get before replay drops it: half again
/// [`DELIVERY_HORIZON`], the ratio Signal Desktop keeps (45 days against
/// its server's 30-day queue, `getAllUnprocessedIds`), so a message that
/// arrived at the end of the horizon still has time to apply.
pub const UNPROCESSED_MAX_AGE: Duration = Duration::from_secs(DELIVERY_HORIZON.as_secs() * 3 / 2);

/// Replays a row gets before it is dropped as one that cannot apply
/// (Signal Desktop's `MAX_UNPROCESSED_ATTEMPTS`).
pub const MAX_UNPROCESSED_ATTEMPTS: u32 = 10;

/// A message's key: its sender and its ratchet header `(dh, n)`, unique
/// per message per session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnprocessedKey {
    pub peer_identity: [u8; 32],
    pub header_dh: [u8; 32],
    pub header_n: u32,
}

/// One decrypted message waiting for its apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unprocessed {
    pub key: UnprocessedKey,
    /// The `DmFrame` kind it arrived as.
    pub type_id: u16,
    pub plaintext: Vec<u8>,
    /// Replays so far, this one included.
    pub attempts: u32,
    /// Receipt time, milliseconds.
    pub received_at: i64,
}

/// Record a decrypted message (inside the ratchet's transaction).
pub fn insert(
    conn: &Connection,
    owner: &[u8; 32],
    key: &UnprocessedKey,
    type_id: u16,
    plaintext: &[u8],
    received_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO unprocessed (owner_key, peer_identity, header_dh, header_n, type_id, \
         plaintext, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            identity_text(owner),
            identity_text(&key.peer_identity),
            key.header_dh,
            key.header_n,
            type_id,
            plaintext,
            received_at
        ],
    )?;
    Ok(())
}

/// The startup replay: drop the rows older than [`UNPROCESSED_MAX_AGE`] or
/// tried [`MAX_UNPROCESSED_ATTEMPTS`] times, count one more attempt on the
/// rest and return them oldest first, in one transaction. A row that does
/// not read back is quarantined (`quarantined_at = now_ms`): no later
/// replay returns it, the age bound sweeps it, and the others replay
/// (audit 2026-10-20 D-9; Signal Desktop likewise drops an unprocessed
/// envelope it cannot parse instead of stopping the queue).
///
/// # Errors
/// The query failed.
pub fn take_for_replay(
    conn: &Connection,
    owner: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<Vec<Unprocessed>> {
    let owner = identity_text(owner);
    let max_age_ms = i64::try_from(UNPROCESSED_MAX_AGE.as_millis()).expect("days fit i64 ms");
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM unprocessed WHERE owner_key = ?1 \
         AND (received_at < ?2 OR attempts >= ?3 OR quarantined_at < ?2)",
        params![
            owner,
            now_ms.saturating_sub(max_age_ms),
            MAX_UNPROCESSED_ATTEMPTS
        ],
    )?;
    tx.execute(
        "UPDATE unprocessed SET attempts = attempts + 1 \
         WHERE owner_key = ?1 AND quarantined_at IS NULL",
        params![owner],
    )?;
    let read_back = {
        let mut stmt = tx.prepare(
            "SELECT peer_identity, header_dh, header_n, type_id, plaintext, attempts, \
             received_at, rowid FROM unprocessed WHERE owner_key = ?1 \
             AND quarantined_at IS NULL ORDER BY received_at, rowid",
        )?;
        let rows = stmt
            .query_map(params![owner], |row| Ok((row.get::<_, i64>(7)?, read(row))))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut rows = Vec::with_capacity(read_back.len());
    let mut quarantined = 0_usize;
    for (rowid, row) in read_back {
        if let Ok(row) = row {
            rows.push(row);
        } else {
            tx.execute(
                "UPDATE unprocessed SET quarantined_at = ?1 WHERE rowid = ?2",
                params![now_ms, rowid],
            )?;
            quarantined += 1;
        }
    }
    tx.commit()?;
    if quarantined > 0 {
        tracing::warn!(quarantined, "unprocessed rows unreadable; quarantined");
    }
    Ok(rows)
}

/// The apply is done: drop the row.
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner: &[u8; 32], key: &UnprocessedKey) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM unprocessed WHERE owner_key = ?1 AND peer_identity = ?2 \
         AND header_dh = ?3 AND header_n = ?4",
        params![
            identity_text(owner),
            identity_text(&key.peer_identity),
            key.header_dh,
            key.header_n
        ],
    )?;
    Ok(())
}

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Unprocessed> {
    let peer: String = row.get(0)?;
    Ok(Unprocessed {
        key: UnprocessedKey {
            peer_identity: super::identity_from_text(&peer, 0)?,
            header_dh: row.get(1)?,
            header_n: row.get(2)?,
        },
        type_id: row.get(3)?,
        plaintext: row.get(4)?,
        attempts: u32::try_from(row::get_u64(row, 5)?)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, i64::MAX))?,
        received_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture;

    const OWNER: [u8; 32] = [1; 32];
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    fn key(n: u32) -> UnprocessedKey {
        UnprocessedKey {
            peer_identity: [2; 32],
            header_dh: [3; 32],
            header_n: n,
        }
    }

    /// D24 / R3-X-1: every retention bound on the delivery path derives
    /// from `DELIVERY_HORIZON` and outlasts it — a message that arrives at
    /// the end of the horizon still finds its prekey (grace ≥ inbox
    /// retention) and still has time to apply (unprocessed age > horizon).
    #[test]
    fn the_retention_bounds_outlast_the_horizon() {
        use rekindle_crypto::signal::prekey_record::RETIRED_ARCHIVE_MS;
        let horizon_ms = i64::try_from(DELIVERY_HORIZON.as_millis()).unwrap();
        assert!(RETIRED_ARCHIVE_MS >= horizon_ms);
        assert!(UNPROCESSED_MAX_AGE > DELIVERY_HORIZON);
        assert_eq!(UNPROCESSED_MAX_AGE, DELIVERY_HORIZON * 3 / 2);
        assert_eq!(UNPROCESSED_MAX_AGE.as_secs(), 45 * 24 * 60 * 60);
    }

    /// Audit D-9: a row that does not read back is set aside, and the
    /// others still replay; the age bound sweeps it.
    #[test]
    fn a_corrupt_row_is_quarantined_and_the_rest_replay() {
        let conn = fixture::identity_conn(&OWNER);
        insert(&conn, &OWNER, &key(0), 1, b"good", 5).unwrap();
        conn.execute(
            "INSERT INTO unprocessed (owner_key, peer_identity, header_dh, header_n, type_id, \
             plaintext, received_at) VALUES (?1, 'not-a-key', zeroblob(32), 9, 1, x'00', 4)",
            params![identity_text(&OWNER)],
        )
        .unwrap();
        let replay = take_for_replay(&conn, &OWNER, 10).unwrap();
        assert_eq!(replay.iter().map(|r| r.key).collect::<Vec<_>>(), [key(0)]);
        let quarantined = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT count(*) FROM unprocessed WHERE quarantined_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(quarantined(&conn), 1);
        assert_eq!(take_for_replay(&conn, &OWNER, 11).unwrap().len(), 1);
        let later = 10 + i64::try_from(UNPROCESSED_MAX_AGE.as_millis()).unwrap() + 1;
        delete(&conn, &OWNER, &key(0)).unwrap();
        assert!(take_for_replay(&conn, &OWNER, later).unwrap().is_empty());
        assert_eq!(quarantined(&conn), 0);
    }

    /// Audit D-9: the header's DH key is 32 bytes and the counters are
    /// non-negative.
    #[test]
    fn malformed_rows_are_refused() {
        let conn = fixture::identity_conn(&OWNER);
        let owner = identity_text(&OWNER);
        let peer = identity_text(&[2; 32]);
        for (dh, n, attempts, at) in [
            ("zeroblob(31)", "0", "0", "0"),
            ("'0123456789abcdef0123456789abcdef'", "0", "0", "0"),
            ("zeroblob(32)", "-1", "0", "0"),
            ("zeroblob(32)", "4294967296", "0", "0"),
            ("zeroblob(32)", "0", "-1", "0"),
            ("zeroblob(32)", "0", "0", "-1"),
        ] {
            let sql = format!(
                "INSERT INTO unprocessed (owner_key, peer_identity, header_dh, header_n, \
                 type_id, plaintext, attempts, received_at) \
                 VALUES (?1, ?2, {dh}, {n}, 1, x'00', {attempts}, {at})"
            );
            assert!(conn.execute(&sql, params![owner, peer]).is_err(), "{sql}");
        }
    }

    #[test]
    fn a_row_replays_until_applied() {
        let conn = fixture::identity_conn(&OWNER);
        insert(&conn, &OWNER, &key(0), 0x0100, b"hello", 5).unwrap();
        insert(&conn, &OWNER, &key(1), 0x0100, b"again", 6).unwrap();
        let replay = take_for_replay(&conn, &OWNER, 10).unwrap();
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].key, key(0));
        assert_eq!(
            (replay[0].plaintext.as_slice(), replay[0].type_id),
            (&b"hello"[..], 0x0100)
        );
        assert_eq!(replay[0].attempts, 1);
        delete(&conn, &OWNER, &key(0)).unwrap();
        let replay = take_for_replay(&conn, &OWNER, 10).unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!((replay[0].key, replay[0].attempts), (key(1), 2));
    }

    #[test]
    fn a_header_is_recorded_once() {
        let conn = fixture::identity_conn(&OWNER);
        insert(&conn, &OWNER, &key(0), 1, b"a", 0).unwrap();
        assert!(insert(&conn, &OWNER, &key(0), 1, b"a", 0).is_err());
    }

    #[test]
    fn stale_and_exhausted_rows_are_dropped_at_replay() {
        let conn = fixture::identity_conn(&OWNER);
        let now = 100 * DAY_MS;
        insert(&conn, &OWNER, &key(0), 1, b"old", now - 46 * DAY_MS).unwrap();
        insert(&conn, &OWNER, &key(1), 1, b"recent", now - 44 * DAY_MS).unwrap();
        let replay = take_for_replay(&conn, &OWNER, now).unwrap();
        assert_eq!(replay.iter().map(|r| r.key).collect::<Vec<_>>(), [key(1)]);
        for _ in 1..MAX_UNPROCESSED_ATTEMPTS {
            assert_eq!(take_for_replay(&conn, &OWNER, now).unwrap().len(), 1);
        }
        assert!(take_for_replay(&conn, &OWNER, now).unwrap().is_empty());
    }
}
