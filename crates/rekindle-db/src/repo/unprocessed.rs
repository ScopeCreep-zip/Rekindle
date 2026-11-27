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

/// Name the message stored from row `key` by its `client_id`, in the
/// transaction that stores it (plan E7.1): its expiry then deletes a row
/// a crash left behind, so a replay cannot bring the message back.
///
/// # Errors
/// The write failed.
pub fn tag(
    conn: &Connection,
    owner: &[u8; 32],
    key: &UnprocessedKey,
    client_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE unprocessed SET client_id = ?5 WHERE owner_key = ?1 AND peer_identity = ?2 \
         AND header_dh = ?3 AND header_n = ?4",
        params![
            identity_text(owner),
            identity_text(&key.peer_identity),
            key.header_dh,
            key.header_n,
            client_id
        ],
    )?;
    Ok(())
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
#[path = "unprocessed_tests.rs"]
mod tests;
