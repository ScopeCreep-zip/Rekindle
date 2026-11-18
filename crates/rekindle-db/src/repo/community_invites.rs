//! `community_invites` — the invites this identity created.

use rusqlite::{params, Connection};

/// Store an invite this identity created.
///
/// # Errors
/// The insert failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    code: &str,
    code_hash: &str,
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO community_invites (owner_key, community_id, code, code_hash, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![owner_key, community_id, code, code_hash, created_at],
    )?;
    Ok(())
}

/// Delete our invite whose code hashes to `code_hash` (it expired).
///
/// # Errors
/// The delete failed.
pub fn delete(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    code_hash: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM community_invites \
         WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3",
        params![owner_key, community_id, code_hash],
    )
}

/// An invite this identity minted, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnInvite {
    pub code: String,
    pub code_hash: String,
    pub created_at: i64,
}

/// Every invite this identity minted for `community_id`, by code hash.
///
/// # Errors
/// The read failed.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<OwnInvite>> {
    let mut stmt = conn.prepare(
        "SELECT code, code_hash, created_at FROM community_invites \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY code_hash",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id], |row| {
        Ok(OwnInvite {
            code: row.get(0)?,
            code_hash: row.get(1)?,
            created_at: row.get(2)?,
        })
    })?;
    rows.collect()
}
