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
/// rest and return them oldest first, in one transaction.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn take_for_replay(
    conn: &Connection,
    owner: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<Vec<Unprocessed>> {
    let owner = identity_text(owner);
    let max_age_ms = i64::try_from(UNPROCESSED_MAX_AGE.as_millis()).expect("days fit i64 ms");
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM unprocessed WHERE owner_key = ?1 AND (received_at < ?2 OR attempts >= ?3)",
        params![
            owner,
            now_ms.saturating_sub(max_age_ms),
            MAX_UNPROCESSED_ATTEMPTS
        ],
    )?;
    tx.execute(
        "UPDATE unprocessed SET attempts = attempts + 1 WHERE owner_key = ?1",
        params![owner],
    )?;
    let rows = {
        let mut stmt = tx.prepare(
            "SELECT peer_identity, header_dh, header_n, type_id, plaintext, attempts, \
             received_at FROM unprocessed WHERE owner_key = ?1 ORDER BY received_at, rowid",
        )?;
        let rows = stmt
            .query_map(params![owner], read)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.commit()?;
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

    #[test]
    fn the_age_bound_outlasts_the_horizon() {
        assert!(UNPROCESSED_MAX_AGE > DELIVERY_HORIZON);
        assert_eq!(UNPROCESSED_MAX_AGE.as_secs(), 45 * 24 * 60 * 60);
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
