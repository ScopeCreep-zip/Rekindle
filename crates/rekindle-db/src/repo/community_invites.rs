//! `community_invites` — the invites this identity created.

use crate::row::{query_list, FromRow};
use rusqlite::{params, Connection, OptionalExtension};

/// Store an invite this identity created.
///
/// # Errors
/// The insert failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    (code, code_hash, url): (&str, &str, &str),
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO community_invites (owner_key, community_id, code, code_hash, url, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![owner_key, community_id, code, code_hash, url, created_at],
    )?;
    Ok(())
}

/// The code of our invite whose code hashes to `code_hash`: the key a
/// joiner's pre-join request is tagged under (plan E3.2 S7c.8 (2)).
///
/// # Errors
/// The read failed.
pub fn code(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    code_hash: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT code FROM community_invites \
         WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3",
        params![owner_key, community_id, code_hash],
        |row| row.get(0),
    )
    .optional()
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
    /// The link as minted (plan IV1b).
    pub url: String,
    pub created_at: i64,
}

impl FromRow for OwnInvite {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(OwnInvite {
            code: row.get(0)?,
            code_hash: row.get(1)?,
            url: row.get(2)?,
            created_at: row.get(3)?,
        })
    }
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
    query_list(
        conn,
        "SELECT code, code_hash, url, created_at FROM community_invites \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY code_hash",
        params![owner_key, community_id],
    )
}
