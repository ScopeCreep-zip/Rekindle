//! `community_invites` — the invites this identity created.

use rusqlite::{params, Connection, OptionalExtension};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_uses_counts_our_invite_and_ignores_others() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE community_invites (owner_key TEXT, community_id TEXT, code_hash TEXT, uses INTEGER NOT NULL DEFAULT 0);
             INSERT INTO community_invites VALUES ('me', 'c', 'h', 0);",
        )
        .unwrap();
        assert_eq!(bump_uses(&conn, "me", "c", "h").unwrap(), Some(1));
        assert_eq!(bump_uses(&conn, "me", "c", "h").unwrap(), Some(2));
        assert_eq!(bump_uses(&conn, "me", "c", "other").unwrap(), None);
    }
}
