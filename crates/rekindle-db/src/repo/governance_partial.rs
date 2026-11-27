//! `governance_partial` (plan E3.2 S7c.8 (1)): a community joined from a
//! checkpoint hint is partial until a rebuild holds every op the hinted
//! walk skipped (Synapse `is_partial_state_room`). The row survives a
//! restart, so a node stopped mid-backfill comes back partial.

use rusqlite::{params, Connection, OptionalExtension};

use crate::repo::governance_ops::{hash32, int};
use crate::row::get_u64;

/// The hint a partial community was joined from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialRow {
    /// The hint's checkpoint height.
    pub height: u64,
    /// The hint's checkpoint digest.
    pub digest: [u8; 32],
    /// The hinted snapshot's digest (the checkpoint's `stateRoot`).
    pub state_root: [u8; 32],
}

/// Mark `community_id` partial, joined from `row`'s hint.
///
/// # Errors
/// The write failed (no such community among ours).
pub fn set(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    row: &PartialRow,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO governance_partial (owner_key, community_id, height, digest, state_root) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(owner_key, community_id) DO UPDATE SET \
         height = excluded.height, digest = excluded.digest, state_root = excluded.state_root",
        params![
            owner_key,
            community_id,
            int(row.height)?,
            row.digest.as_slice(),
            row.state_root.as_slice(),
        ],
    )?;
    Ok(())
}

/// The hint `community_id` is partial from, `None` once it is whole.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn get(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Option<PartialRow>> {
    conn.query_row(
        "SELECT height, digest, state_root FROM governance_partial \
         WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
        |row| {
            Ok(PartialRow {
                height: get_u64(row, 0)?,
                digest: hash32(row, "digest")?,
                state_root: hash32(row, "state_root")?,
            })
        },
    )
    .optional()
}

/// `community_id` is whole: its backfill ended.
///
/// # Errors
/// The write failed.
pub fn clear(conn: &Connection, owner_key: &str, community_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM governance_partial WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "governance_partial_tests.rs"]
mod tests;
