//! `recent_servers` (plan E7.2 G3, SK2-G §3): the game servers we ourselves
//! visited, a game and a `ServerAddress` text (host:port, parsed before it
//! is stored) with the last visit. Only ours: never sent. At most
//! [`MAX_RECENT`] rows; a new visit past that drops the oldest.

use rusqlite::{params, Connection};

/// How many recent servers we keep (SK2-G §3).
pub const MAX_RECENT: u32 = 50;

/// One visited server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentServer {
    pub game_id: u32,
    pub address: String,
    /// Unix seconds.
    pub last_seen: i64,
}

/// Record a visit to `address` for `game_id` at `last_seen`, then keep the
/// newest [`MAX_RECENT`] rows.
///
/// # Errors
/// The write failed or `game_id` is 0 (the table's check).
pub fn touch(conn: &Connection, owner_key: &str, recent: &RecentServer) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO recent_servers (owner_key, game_id, address, last_seen) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(owner_key, game_id, address) DO UPDATE SET last_seen = excluded.last_seen",
        params![owner_key, recent.game_id, recent.address, recent.last_seen],
    )?;
    conn.execute(
        "DELETE FROM recent_servers WHERE owner_key = ?1 AND rowid NOT IN \
         (SELECT rowid FROM recent_servers WHERE owner_key = ?1 \
          ORDER BY last_seen DESC, game_id, address LIMIT ?2)",
        params![owner_key, MAX_RECENT],
    )?;
    Ok(())
}

/// Forget `address` for `game_id`; forgetting one not there changes nothing.
///
/// # Errors
/// The write failed.
pub fn remove(
    conn: &Connection,
    owner_key: &str,
    game_id: u32,
    address: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM recent_servers WHERE owner_key = ?1 AND game_id = ?2 AND address = ?3",
        params![owner_key, game_id, address],
    )?;
    Ok(())
}

/// Forget every recent server (game detection turned off).
///
/// # Errors
/// The write failed.
pub fn clear(conn: &Connection, owner_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM recent_servers WHERE owner_key = ?1",
        params![owner_key],
    )?;
    Ok(())
}

/// Our recent servers, the newest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<RecentServer>> {
    let mut stmt = conn.prepare(
        "SELECT game_id, address, last_seen FROM recent_servers \
         WHERE owner_key = ?1 ORDER BY last_seen DESC, game_id, address",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(RecentServer {
            game_id: row.get(0)?,
            address: row.get(1)?,
            last_seen: row.get(2)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn visit(game_id: u32, address: &str, last_seen: i64) -> RecentServer {
        RecentServer {
            game_id,
            address: address.to_owned(),
            last_seen,
        }
    }

    #[test]
    fn a_revisit_moves_the_row_and_the_oldest_past_the_cap_goes() {
        let conn = fixture::conn();
        let cap = usize::try_from(MAX_RECENT).unwrap();
        for i in 0..i64::from(MAX_RECENT) {
            touch(
                &conn,
                OWNER,
                &visit(10, &format!("1.2.3.4:{}", 1000 + i), i),
            )
            .unwrap();
        }
        touch(&conn, OWNER, &visit(10, "1.2.3.4:1000", 500)).unwrap();
        touch(&conn, OWNER, &visit(11, "5.6.7.8:1", 600)).unwrap();
        let rows = list(&conn, OWNER).unwrap();
        assert_eq!(rows.len(), cap);
        assert_eq!(rows[0], visit(11, "5.6.7.8:1", 600));
        assert_eq!(rows[1], visit(10, "1.2.3.4:1000", 500));
        assert!(
            !rows.iter().any(|r| r.address == "1.2.3.4:1001"),
            "the oldest stays"
        );
        remove(&conn, OWNER, 11, "5.6.7.8:1").unwrap();
        assert_eq!(list(&conn, OWNER).unwrap().len(), cap - 1);
        clear(&conn, OWNER).unwrap();
        assert!(list(&conn, OWNER).unwrap().is_empty());
    }
}
