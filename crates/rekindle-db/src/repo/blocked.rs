//! `blocked_users`: the identities we refuse (plan E2.1). A blocked sender
//! reaches nothing: the 1:1 receive gate drops it before any handler.

use rusqlite::{params, Connection, OptionalExtension as _};

/// A blocked identity, newest first in [`list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEntry {
    pub public_key: String,
    pub display_name: String,
    pub blocked_at: i64,
}

/// Whether `public_key` is blocked.
///
/// # Errors
/// The query failed.
pub fn is_blocked(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
            params![owner_key, public_key],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Block `public_key` (a repeat updates the name and time).
///
/// # Errors
/// The write failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    blocked_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO blocked_users (owner_key, public_key, display_name, blocked_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![owner_key, public_key, display_name, blocked_at],
    )?;
    Ok(())
}

/// Unblock `public_key`; whether it was blocked.
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key],
    )? > 0)
}

/// Every blocked identity, newest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<BlockEntry>> {
    let mut stmt = conn.prepare(
        "SELECT public_key, display_name, blocked_at FROM blocked_users \
         WHERE owner_key = ?1 ORDER BY blocked_at DESC",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(BlockEntry {
                public_key: row.get(0)?,
                display_name: row.get(1)?,
                blocked_at: row.get(2)?,
            })
        })?
        .collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_block_is_listed_until_it_is_lifted() {
        let conn = fixture::conn();
        assert!(!is_blocked(&conn, OWNER, "eve").unwrap());
        insert(&conn, OWNER, "eve", "Eve", 5).unwrap();
        insert(&conn, OWNER, "mallory", "Mallory", 9).unwrap();
        assert!(is_blocked(&conn, OWNER, "eve").unwrap());
        let listed: Vec<String> = list(&conn, OWNER)
            .unwrap()
            .into_iter()
            .map(|b| b.public_key)
            .collect();
        assert_eq!(listed, ["mallory", "eve"]);
        assert!(delete(&conn, OWNER, "eve").unwrap());
        assert!(!delete(&conn, OWNER, "eve").unwrap());
        assert!(!is_blocked(&conn, OWNER, "eve").unwrap());
    }
}
