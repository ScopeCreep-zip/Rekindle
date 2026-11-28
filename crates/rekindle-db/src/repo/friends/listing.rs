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

/// Whether `peer` is `owner_key`'s friend, or a handshake with them is
/// open: our request to them waits for their accept (`pending_out`), or
/// theirs waits in `pending_friend_requests` for our answer (plan RV2). Only such
/// a peer's evidence about our route counts; a peer we are removing does
/// not.
///
/// # Errors
/// The query failed.
pub fn open_handshake_or_friend(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM friends WHERE owner_key = ?1 AND public_key = ?2 \
           AND friendship_state IN ('accepted', 'pending_out')) \
         OR EXISTS (SELECT 1 FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2)",
    )?
    .query_row(params![owner_key, peer], |row| row.get(0))
}
