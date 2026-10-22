//! The threads a community holds at receipt because their creator was
//! timed out (plan E3.1 F3h): the merge reads no clock, so the moment we
//! receive a thread is the only time it can be judged, and the hold is
//! kept here so the unlock loader re-applies it after a restart.

use rusqlite::{params, Connection};

/// Hold `thread_ids` (lowercase hex) of `community_id` from `held_at`;
/// a thread already held keeps its first time.
///
/// # Errors
/// An insert failed.
pub fn hold(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    thread_ids: &[String],
    held_at: u64,
) -> rusqlite::Result<()> {
    let held_at =
        i64::try_from(held_at).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, 0))?;
    let mut stmt = conn.prepare_cached(
        "INSERT OR IGNORE INTO community_held_threads \
         (owner_key, community_id, thread_id, held_at) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for thread_id in thread_ids {
        stmt.execute(params![owner_key, community_id, thread_id, held_at])?;
    }
    Ok(())
}

/// Every held thread of `owner_key` as `(community_id, thread_id)`, in
/// that order.
///
/// # Errors
/// The query failed.
pub fn load(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, thread_id FROM community_held_threads \
         WHERE owner_key = ?1 ORDER BY community_id, thread_id",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_hold_is_kept_once_and_cascades_with_its_community() {
        let conn = fixture::conn();
        fixture::community(&conn, "c");
        let id = "0a".repeat(16);
        hold(&conn, OWNER, "c", &[id.clone(), id.clone()], 5).unwrap();
        hold(&conn, OWNER, "c", std::slice::from_ref(&id), 9).unwrap();
        assert_eq!(load(&conn, OWNER).unwrap(), vec![("c".to_owned(), id)]);
        conn.execute("DELETE FROM communities WHERE id = 'c'", [])
            .unwrap();
        assert!(load(&conn, OWNER).unwrap().is_empty());
    }
}
