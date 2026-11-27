//! `presence_keys`: our friend-presence key (plan E2.5), the newest epoch
//! only.

use crate::row::query_opt;
use rusqlite::{params, Connection};

/// Our current key: `(epoch, key)`, or `None` before the first is minted.
///
/// # Errors
/// The query failed.
pub fn current(conn: &Connection, owner_key: &str) -> rusqlite::Result<Option<(u32, Vec<u8>)>> {
    query_opt(
        conn,
        "SELECT epoch, key FROM presence_keys WHERE owner_key = ?1 \
         ORDER BY epoch DESC LIMIT 1",
        params![owner_key],
    )
}

/// Store the key of `epoch` as our current one; the older epochs go.
///
/// # Errors
/// A write failed.
pub fn replace(
    conn: &Connection,
    owner_key: &str,
    epoch: u32,
    key: &[u8],
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO presence_keys (owner_key, epoch, key, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![owner_key, epoch, key, created_at],
    )?;
    crate::erase::delete(
        conn,
        "DELETE FROM presence_keys WHERE owner_key = ?1 AND epoch < ?2",
        params![owner_key, epoch],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "presence_keys_tests.rs"]
mod tests;
