//! `governance_ops` and `governance_heads` (plan E3.2, ADR 0011): every
//! signed governance op this identity holds for a community, with its fate
//! in the last merge, and each device key's newest op (ADR 0022: ops are
//! signed by per-community device keys, one chain and one member record
//! per key). One repo for both
//! tracks (D1): `rebuild_governance` re-merges the op set from here, and
//! the unlock's warm load reads it before the network answers.
//!
//! An op is stored by its hash and never rewritten: its bytes are what its
//! author signed. Only its `state` moves.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::row::{get, get_enum, get_u64};

/// An op's fate in the last merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    /// Verified and merged (or refused by validation, which is still a
    /// complete op every reader agrees on).
    Applied,
    /// A dep or the author's previous op is not held yet.
    PendingDeps,
    /// Undecodable or failed verification; kept so it is not fetched again.
    Quarantined,
    /// Its author signed two ops at one seq.
    Equivocation,
}

impl OpState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::PendingDeps => "pending_deps",
            Self::Quarantined => "quarantined",
            Self::Equivocation => "equivocation",
        }
    }
}

impl std::str::FromStr for OpState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "applied" => Ok(Self::Applied),
            "pending_deps" => Ok(Self::PendingDeps),
            "quarantined" => Ok(Self::Quarantined),
            "equivocation" => Ok(Self::Equivocation),
            other => Err(format!("unknown governance op state {other:?}")),
        }
    }
}

/// One stored op: its encoded bytes (`rekindle_codec::governance_op`) and
/// the columns a reader selects and orders by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOp {
    pub op_hash: [u8; 32],
    /// The author pseudonym, hex.
    pub author: String,
    /// The key that signed the op, hex (ADR 0022 rule 1).
    pub device_key: String,
    /// The op's seq in `device_key`'s chain.
    pub device_seq: u64,
    pub lamport: u64,
    pub issued_at: u64,
    pub op: Vec<u8>,
    pub state: OpState,
    /// This device signed the op (the local signed log, plan S7d-keys.9);
    /// never set for an op ingested by gossip, the DAG fetch or the
    /// barrier's walk.
    pub signed_here: bool,
}

/// A device key's newest op seen, and the member record its chain lives
/// in (ADR 0022 rule 3: one record per device key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceHead {
    /// The device key, hex.
    pub device_key: String,
    /// The pseudonym the key signs for, hex.
    pub author: String,
    pub member_record: String,
    pub head_seq: u64,
    pub frontier_hash: [u8; 32],
}

pub(crate) fn int(value: u64) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

pub(crate) fn hash32(row: &Row<'_>, column: &str) -> rusqlite::Result<[u8; 32]> {
    let bytes: Vec<u8> = get(row, column)?;
    bytes.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("{column} is not 32 bytes").into(),
        )
    })
}

const OP_COLUMNS: &str =
    "op_hash, author, device_key, device_seq, lamport, issued_at, op, state, signed_here";

/// The op whose columns start at `at` ([`OP_COLUMNS`] order).
fn op_at(row: &Row<'_>, at: usize) -> rusqlite::Result<StoredOp> {
    Ok(StoredOp {
        op_hash: hash32(row, "op_hash")?,
        author: get(row, "author")?,
        device_key: get(row, "device_key")?,
        device_seq: get_u64(row, at + 3)?,
        lamport: get_u64(row, at + 4)?,
        issued_at: get_u64(row, at + 5)?,
        op: get(row, "op")?,
        state: get_enum(row, "state")?,
        signed_here: get(row, "signed_here")?,
    })
}

fn op_row(row: &Row<'_>) -> rusqlite::Result<StoredOp> {
    op_at(row, 0)
}

/// Store `op` for `community_id`. An op already held keeps its row (its
/// bytes are fixed by its hash); returns whether this call stored it.
///
/// # Errors
/// The write failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    op: &StoredOp,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "INSERT INTO governance_ops (owner_key, community_id, op_hash, author, device_key, \
         device_seq, lamport, issued_at, op, state, signed_here) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(owner_key, community_id, op_hash) DO NOTHING",
        params![
            owner_key,
            community_id,
            op.op_hash.as_slice(),
            op.author,
            op.device_key,
            int(op.device_seq)?,
            int(op.lamport)?,
            int(op.issued_at)?,
            op.op,
            op.state.as_str(),
            op.signed_here,
        ],
    )?;
    Ok(changed == 1)
}

/// One op by hash.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn get_op(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    op_hash: &[u8; 32],
) -> rusqlite::Result<Option<StoredOp>> {
    conn.query_row(
        &format!(
            "SELECT {OP_COLUMNS} FROM governance_ops \
             WHERE owner_key = ?1 AND community_id = ?2 AND op_hash = ?3"
        ),
        params![owner_key, community_id, op_hash.as_slice()],
        op_row,
    )
    .optional()
}

/// `community_id`'s ops, in `state` when given, by `(lamport, op_hash)`.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    state: Option<OpState>,
) -> rusqlite::Result<Vec<StoredOp>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {OP_COLUMNS} FROM governance_ops \
         WHERE owner_key = ?1 AND community_id = ?2 AND (?3 IS NULL OR state = ?3) \
         ORDER BY lamport, op_hash"
    ))?;
    let rows = stmt.query_map(
        params![owner_key, community_id, state.map(OpState::as_str)],
        op_row,
    )?;
    rows.collect()
}

/// Every community's ops for `owner_key`, `(community_id, op)` by
/// community then `(lamport, op_hash)`: the unlock's warm load.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn load_all(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<(String, StoredOp)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT community_id, {OP_COLUMNS} FROM governance_ops WHERE owner_key = ?1 \
         ORDER BY community_id, lamport, op_hash"
    ))?;
    let rows = stmt.query_map(params![owner_key], |row| {
        let community: String = get(row, "community_id")?;
        Ok((community, op_at(row, 1)?))
    })?;
    rows.collect()
}

/// Record the fate of `op_hash` in the last merge and, for an op the merge
/// verified, its validated lamport (`1 + max(dep lamports)`); `None` keeps
/// the stored lamport (an op that did not verify has none).
///
/// # Errors
/// The write failed.
pub fn set_state(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    op_hash: &[u8; 32],
    state: OpState,
    lamport: Option<u64>,
) -> rusqlite::Result<()> {
    let lamport = lamport.map(int).transpose()?;
    conn.execute(
        "UPDATE governance_ops SET state = ?4, lamport = COALESCE(?5, lamport) \
         WHERE owner_key = ?1 AND community_id = ?2 AND op_hash = ?3",
        params![
            owner_key,
            community_id,
            op_hash.as_slice(),
            state.as_str(),
            lamport
        ],
    )?;
    Ok(())
}

/// Every device key's head in `community_id`, by device key.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn heads(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<DeviceHead>> {
    let mut stmt = conn.prepare(
        "SELECT device_key, author, member_record, head_seq, frontier_hash FROM governance_heads \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY device_key",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id], |row| {
        Ok(DeviceHead {
            device_key: get(row, "device_key")?,
            author: get(row, "author")?,
            member_record: get(row, "member_record")?,
            head_seq: get_u64(row, 3)?,
            frontier_hash: hash32(row, "frontier_hash")?,
        })
    })?;
    rows.collect()
}

/// The highest `device_seq` this device signed under `device_key` in
/// `community_id`: `signed_here` rows only (plan E3.2 S7d-keys.3), never
/// our own ops re-learnt from peers, so the unlock barrier (S7d-keys.8)
/// reads a restored copy as behind its key's DHT head.
///
/// # Errors
/// The query failed or the stored seq is negative.
pub fn signed_here_head(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    device_key: &str,
) -> rusqlite::Result<Option<u64>> {
    let seq: Option<i64> = conn.query_row(
        "SELECT MAX(device_seq) FROM governance_ops WHERE owner_key = ?1 AND community_id = ?2 \
         AND device_key = ?3 AND signed_here = 1",
        params![owner_key, community_id, device_key],
        |row| row.get(0),
    )?;
    seq.map(|seq| u64::try_from(seq).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, seq)))
        .transpose()
}

/// Write `head` as its device key's head in `community_id`.
///
/// # Errors
/// The write failed.
pub fn set_head(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    head: &DeviceHead,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO governance_heads (owner_key, community_id, device_key, author, \
         member_record, head_seq, frontier_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(owner_key, community_id, device_key) DO UPDATE SET \
         author = excluded.author, member_record = excluded.member_record, \
         head_seq = excluded.head_seq, frontier_hash = excluded.frontier_hash",
        params![
            owner_key,
            community_id,
            head.device_key,
            head.author,
            head.member_record,
            int(head.head_seq)?,
            head.frontier_hash.as_slice(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn op(byte: u8, lamport: u64, state: OpState) -> StoredOp {
        StoredOp {
            op_hash: [byte; 32],
            author: "aa".into(),
            device_key: "dd".into(),
            device_seq: u64::from(byte),
            lamport,
            issued_at: 7,
            op: vec![byte],
            state,
            signed_here: false,
        }
    }

    #[test]
    fn an_op_is_stored_once_and_lists_by_lamport_and_state() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        assert!(insert(&conn, OWNER, "c1", &op(2, 2, OpState::Applied)).unwrap());
        assert!(insert(&conn, OWNER, "c1", &op(1, 1, OpState::PendingDeps)).unwrap());
        assert!(!insert(&conn, OWNER, "c1", &op(2, 9, OpState::Quarantined)).unwrap());
        let all = list(&conn, OWNER, "c1", None).unwrap();
        assert_eq!(
            all,
            vec![op(1, 1, OpState::PendingDeps), op(2, 2, OpState::Applied)]
        );
        let pending = list(&conn, OWNER, "c1", Some(OpState::PendingDeps)).unwrap();
        assert_eq!(pending, vec![op(1, 1, OpState::PendingDeps)]);
        assert_eq!(load_all(&conn, OWNER).unwrap().len(), 2);
    }

    #[test]
    fn set_state_moves_the_state_and_the_validated_lamport() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        insert(&conn, OWNER, "c1", &op(1, 0, OpState::PendingDeps)).unwrap();
        set_state(&conn, OWNER, "c1", &[1; 32], OpState::Applied, Some(7)).unwrap();
        let got = get_op(&conn, OWNER, "c1", &[1; 32]).unwrap().unwrap();
        assert_eq!(got, op(1, 7, OpState::Applied));
        set_state(&conn, OWNER, "c1", &[1; 32], OpState::Quarantined, None).unwrap();
        let got = get_op(&conn, OWNER, "c1", &[1; 32]).unwrap().unwrap();
        assert_eq!(got, op(1, 7, OpState::Quarantined));
        assert_eq!(get_op(&conn, OWNER, "c1", &[9; 32]).unwrap(), None);
    }

    #[test]
    fn a_head_is_replaced_per_device_key() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        let head = |device: &str, record: &str, seq| DeviceHead {
            device_key: device.into(),
            author: "aa".into(),
            member_record: record.into(),
            head_seq: seq,
            frontier_hash: [3; 32],
        };
        set_head(&conn, OWNER, "c1", &head("d1", "VLD0:m1", 1)).unwrap();
        set_head(&conn, OWNER, "c1", &head("d1", "VLD0:m1", 4)).unwrap();
        // A second device of the same author keeps its own head and record
        // (ADR 0022 rule 3).
        set_head(&conn, OWNER, "c1", &head("d2", "VLD0:m2", 0)).unwrap();
        assert_eq!(
            heads(&conn, OWNER, "c1").unwrap(),
            vec![head("d1", "VLD0:m1", 4), head("d2", "VLD0:m2", 0)]
        );
    }

    #[test]
    fn signed_here_is_stored_and_a_re_insert_never_changes_it() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        let mine = StoredOp {
            signed_here: true,
            ..op(1, 1, OpState::Applied)
        };
        assert!(insert(&conn, OWNER, "c1", &mine).unwrap());
        // The same op back by gossip keeps the local signer's flag.
        assert!(!insert(&conn, OWNER, "c1", &op(1, 1, OpState::Applied)).unwrap());
        assert_eq!(get_op(&conn, OWNER, "c1", &[1; 32]).unwrap(), Some(mine));
        insert(&conn, OWNER, "c1", &op(2, 2, OpState::Applied)).unwrap();
        let loaded: Vec<bool> = load_all(&conn, OWNER)
            .unwrap()
            .into_iter()
            .map(|(_, op)| op.signed_here)
            .collect();
        assert_eq!(loaded, vec![true, false]);
    }

    #[test]
    fn the_signed_here_head_never_counts_ops_re_learnt_from_peers() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        assert_eq!(signed_here_head(&conn, OWNER, "c1", "dd").unwrap(), None);
        let mine = |byte| StoredOp {
            signed_here: true,
            ..op(byte, u64::from(byte), OpState::Applied)
        };
        insert(&conn, OWNER, "c1", &mine(1)).unwrap();
        insert(&conn, OWNER, "c1", &mine(2)).unwrap();
        // Our key's later ops, echoed back by a peer: signed by another copy.
        insert(&conn, OWNER, "c1", &op(5, 5, OpState::Applied)).unwrap();
        insert(&conn, OWNER, "c1", &op(6, 6, OpState::Applied)).unwrap();
        assert_eq!(signed_here_head(&conn, OWNER, "c1", "dd").unwrap(), Some(2));
        assert_eq!(signed_here_head(&conn, OWNER, "c1", "ee").unwrap(), None);
    }
}
