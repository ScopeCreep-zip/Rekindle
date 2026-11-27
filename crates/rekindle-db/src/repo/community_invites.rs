//! `community_invites` — the invites this identity created.

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
    inbox: Option<&JoinInbox>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO community_invites
         (owner_key, community_id, code, code_hash, url, created_at, join_inbox_key, join_inbox_invite_id, max_uses, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            owner_key, community_id, code, code_hash, url, created_at,
            inbox.map(|i| i.key.as_str()), inbox.map(|i| i.invite_id.as_str()),
            inbox.map_or(0, |i| i.slots), inbox.and_then(|i| i.expires_at),
        ],
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
         WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3 AND join_inbox_key IS NULL",
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
    pub join_inbox: Option<JoinInbox>,
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
        "SELECT code, code_hash, url, created_at, join_inbox_key, join_inbox_invite_id, max_uses, expires_at FROM community_invites \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY code_hash",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id], |row| {
        Ok(OwnInvite {
            code: row.get(0)?,
            code_hash: row.get(1)?,
            url: row.get(2)?,
            created_at: row.get(3)?,
            join_inbox: match (row.get(4)?, row.get(5)?) {
                (Some(key), Some(invite_id)) => Some(JoinInbox {
                    key,
                    invite_id,
                    slots: row.get(6)?,
                    expires_at: row.get(7)?,
                }),
                _ => None,
            },
        })
    })?;
    rows.collect()
}

/// An invite's owner record, retained until its final read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinInbox {
    pub key: String,
    pub invite_id: String,
    pub slots: u8,
    pub expires_at: Option<u64>,
}

/// Delete a retired invite after its inbox has received its final read.
/// # Errors
/// The update failed.
pub fn retire_inbox(
    conn: &Connection,
    owner: &str,
    community: &str,
    invite_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM community_invites
         WHERE owner_key = ?1 AND community_id = ?2 AND join_inbox_invite_id = ?3",
        params![owner, community, invite_id],
    )?;
    Ok(())
}
