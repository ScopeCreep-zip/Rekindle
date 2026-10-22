//! `community_invites` — the invites this identity created.

use rusqlite::{params, Connection, OptionalExtension};

/// Store an invite this identity created, with no uses yet.
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

/// Count one use of the invite whose code hashes to `code_hash` (a peer's
/// `MemberJoinRequest` redeemed it): the new use count, or `None` when the
/// invite is not ours.
///
/// # Errors
/// The update or read failed.
pub fn bump_uses(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    code_hash: &str,
) -> rusqlite::Result<Option<u32>> {
    conn.query_row(
        "UPDATE community_invites SET uses = uses + 1 \
         WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3 RETURNING uses",
        params![owner_key, community_id, code_hash],
        |row| row.get(0),
    )
    .optional()
}

/// The use count of our invite whose code hashes to `code_hash`.
///
/// # Errors
/// The read failed.
pub fn uses(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    code_hash: &str,
) -> rusqlite::Result<Option<u32>> {
    conn.query_row(
        "SELECT uses FROM community_invites \
         WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3",
        params![owner_key, community_id, code_hash],
        |row| row.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_uses_counts_our_invite_and_ignores_others() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE community_invites (owner_key TEXT, community_id TEXT, code TEXT, \
             code_hash TEXT, created_at INTEGER, uses INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        insert(&conn, "me", "c", "code", "h", 1).unwrap();
        assert_eq!(bump_uses(&conn, "me", "c", "h").unwrap(), Some(1));
        assert_eq!(bump_uses(&conn, "me", "c", "h").unwrap(), Some(2));
        assert_eq!(bump_uses(&conn, "me", "c", "other").unwrap(), None);
        assert_eq!(uses(&conn, "me", "c", "h").unwrap(), Some(2));
    }
}
