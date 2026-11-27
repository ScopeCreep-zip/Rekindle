//! `blocked_users`: the identities we refuse (plan E2.1). A blocked sender
//! reaches nothing: the 1:1 receive gate drops it before any handler.

use crate::row::{query_list, FromRow};
use rusqlite::{params, Connection, OptionalExtension as _};

/// A blocked identity, newest first in [`list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEntry {
    pub public_key: String,
    pub display_name: String,
    pub blocked_at: i64,
}

impl FromRow for BlockEntry {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(BlockEntry {
            public_key: row.get(0)?,
            display_name: row.get(1)?,
            blocked_at: row.get(2)?,
        })
    }
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
    query_list(
        conn,
        "SELECT public_key, display_name, blocked_at FROM blocked_users \
         WHERE owner_key = ?1 ORDER BY blocked_at DESC",
        params![owner_key],
    )
}

#[cfg(test)]
#[path = "blocked_tests.rs"]
mod tests;
