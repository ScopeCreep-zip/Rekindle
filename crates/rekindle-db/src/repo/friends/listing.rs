//! What our friend-list record publishes, read from the rows alone, so a
//! write of it needs no network read first (review RV3, D22).

use rusqlite::{params, Connection};

/// Each stored friend's `added_at`, by public key.
///
/// # Errors
/// The query failed.
pub fn added_at(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare("SELECT public_key, added_at FROM friends WHERE owner_key = ?1")?;
    let rows = stmt
        .query_map(params![owner_key], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect();
    rows
}
