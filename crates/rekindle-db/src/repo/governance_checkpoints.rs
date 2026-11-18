//! `governance_checkpoints` (plan E3.2 S7c.6): every (height, round,
//! digest) a held `Checkpoint` op carries, with its state after the last
//! rebuild, and the verified state snapshot of a usable one. Which hint is
//! selected is computed from the op set, never stored.
//!
//! A row's state moves with each rebuild (a late majority makes a digest
//! `usable`, a second majority digest for the same cut makes both
//! `equivocated`); its snapshot stays, but is read only while the row is
//! `usable`, so a snapshot whose checkpoint is later equivocated is never
//! served again.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::repo::governance_ops::{hash32, int};
use crate::row::{get, get_enum, get_u64};

/// A checkpoint digest's state after the last rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointState {
    /// Carried by `Checkpoint` ops short of a strict majority.
    Proposed,
    /// A strict majority of its electorate endorses it.
    Usable,
    /// One of two majority digests for one cut and one `prevCheckpoint`.
    Equivocated,
}

impl CheckpointState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Usable => "usable",
            Self::Equivocated => "equivocated",
        }
    }
}

impl std::str::FromStr for CheckpointState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "proposed" => Ok(Self::Proposed),
            "usable" => Ok(Self::Usable),
            "equivocated" => Ok(Self::Equivocated),
            other => Err(format!("unknown checkpoint state {other:?}")),
        }
    }
}

/// One checkpoint row, without its snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRow {
    pub height: u64,
    pub round: u32,
    pub digest: [u8; 32],
    /// The lowest `(lamport, op_hash)` op carrying the digest.
    pub op_hash: [u8; 32],
    pub dir_size: u64,
    pub state: CheckpointState,
}

fn checkpoint_row(row: &Row<'_>) -> rusqlite::Result<CheckpointRow> {
    let round = get_u64(row, 1)?;
    Ok(CheckpointRow {
        height: get_u64(row, 0)?,
        round: u32::try_from(round).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Integer, e.into())
        })?,
        digest: hash32(row, "digest")?,
        op_hash: hash32(row, "op_hash")?,
        dir_size: get_u64(row, 4)?,
        state: get_enum(row, "state")?,
    })
}

/// Write `rows` as `community_id`'s checkpoints after a rebuild: each row
/// is inserted or has its op, size and state replaced; a kept snapshot
/// stays.
///
/// # Errors
/// The write failed.
pub fn record(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    rows: &[CheckpointRow],
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO governance_checkpoints (owner_key, community_id, height, round, digest, \
         op_hash, dir_size, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(owner_key, community_id, height, round, digest) DO UPDATE SET \
         op_hash = excluded.op_hash, dir_size = excluded.dir_size, state = excluded.state",
    )?;
    for row in rows {
        stmt.execute(params![
            owner_key,
            community_id,
            int(row.height)?,
            row.round,
            row.digest.as_slice(),
            row.op_hash.as_slice(),
            int(row.dir_size)?,
            row.state.as_str(),
        ])?;
    }
    Ok(())
}

/// `community_id`'s checkpoints by `(height, round, digest)`.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<CheckpointRow>> {
    let mut stmt = conn.prepare(
        "SELECT height, round, digest, op_hash, dir_size, state FROM governance_checkpoints \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY height, round, digest",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id], checkpoint_row)?;
    rows.collect()
}

/// Keep `snapshot`, verified against the checkpoint's `stateRoot`, on the
/// usable row at `height` with `digest`; `false` when no such row is
/// usable (nothing is kept).
///
/// # Errors
/// The write failed.
pub fn set_snapshot(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    height: u64,
    digest: &[u8; 32],
    snapshot: &[u8],
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE governance_checkpoints SET snapshot = ?5 WHERE owner_key = ?1 \
         AND community_id = ?2 AND height = ?3 AND digest = ?4 AND state = 'usable'",
        params![
            owner_key,
            community_id,
            int(height)?,
            digest.as_slice(),
            snapshot
        ],
    )?;
    Ok(changed > 0)
}

/// The kept snapshot at `height` with `digest`, only while its row is
/// `usable`.
///
/// # Errors
/// The query failed.
pub fn usable_snapshot(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    height: u64,
    digest: &[u8; 32],
) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.query_row(
        "SELECT snapshot FROM governance_checkpoints WHERE owner_key = ?1 \
         AND community_id = ?2 AND height = ?3 AND digest = ?4 AND state = 'usable' \
         AND snapshot IS NOT NULL",
        params![owner_key, community_id, int(height)?, digest.as_slice()],
        |row| get(row, "snapshot"),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn row(digest: u8, state: CheckpointState) -> CheckpointRow {
        CheckpointRow {
            height: 1,
            round: u32::from(digest),
            digest: [digest; 32],
            op_hash: [9; 32],
            dir_size: 3,
            state,
        }
    }

    #[test]
    fn a_row_moves_its_state_and_lists_in_order() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        record(&conn, OWNER, "c1", &[row(2, CheckpointState::Proposed)]).unwrap();
        record(
            &conn,
            OWNER,
            "c1",
            &[
                row(1, CheckpointState::Proposed),
                row(2, CheckpointState::Usable),
            ],
        )
        .unwrap();
        assert_eq!(
            list(&conn, OWNER, "c1").unwrap(),
            vec![
                row(1, CheckpointState::Proposed),
                row(2, CheckpointState::Usable)
            ]
        );
    }

    /// Plan E3.2 S7c.6: a snapshot is kept only on a usable row and read
    /// only while the row stays usable; once its checkpoint is equivocated
    /// it is not read, though a later rebuild could make it usable again.
    #[test]
    fn a_snapshot_is_kept_and_read_only_while_its_row_is_usable() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        record(&conn, OWNER, "c1", &[row(1, CheckpointState::Proposed)]).unwrap();
        assert!(!set_snapshot(&conn, OWNER, "c1", 1, &[1; 32], b"s").unwrap());
        record(&conn, OWNER, "c1", &[row(1, CheckpointState::Usable)]).unwrap();
        assert!(set_snapshot(&conn, OWNER, "c1", 1, &[1; 32], b"s").unwrap());
        let read = || usable_snapshot(&conn, OWNER, "c1", 1, &[1; 32]).unwrap();
        assert_eq!(read().as_deref(), Some(b"s".as_slice()));
        record(&conn, OWNER, "c1", &[row(1, CheckpointState::Equivocated)]).unwrap();
        assert_eq!(
            read(),
            None,
            "an equivocated checkpoint's snapshot is not read"
        );
        assert_eq!(
            usable_snapshot(&conn, OWNER, "c1", 1, &[2; 32]).unwrap(),
            None
        );
    }

    /// Plan E3.2 S7c.6: the FK to `communities`.
    #[test]
    fn governance_checkpoints_refuses_an_unknown_community() {
        let conn = fixture::conn();
        let err = record(&conn, OWNER, "gone", &[row(1, CheckpointState::Usable)])
            .unwrap_err()
            .to_string();
        assert!(err.contains("FOREIGN KEY constraint failed"), "{err}");
    }

    /// Plan E3.2 S7c.6: the state CHECK.
    #[test]
    fn governance_checkpoints_refuses_an_unknown_state() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        let err = conn
            .execute(
                "INSERT INTO governance_checkpoints (owner_key, community_id, height, round, \
                 digest, op_hash, dir_size, state) \
                 VALUES (?1, 'c1', 1, 0, zeroblob(32), zeroblob(32), 0, 'adopted')",
                params![OWNER],
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("CHECK constraint failed"), "{err}");
    }
}
