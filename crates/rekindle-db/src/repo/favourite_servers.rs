//! `favourite_servers` (plan E7.2): our personal favourite game servers, a
//! game, a `ServerAddress` text (host:port, parsed before it is stored)
//! and a label of at most 64 characters. Only ours: never sent, though a
//! friend's chat window offers each as a `GameInvite`.

use rusqlite::{params, Connection};

/// One stored favourite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavouriteServer {
    pub game_id: u32,
    pub address: String,
    pub label: String,
    pub added_at: i64,
}

/// Add `address` for `game_id`, or relabel it when it is already a
/// favourite (it keeps its `added_at`).
///
/// # Errors
/// The write failed, `game_id` is 0 or the label is over 64 characters
/// (the table's checks).
pub fn add(
    conn: &Connection,
    owner_key: &str,
    favourite: &FavouriteServer,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO favourite_servers (owner_key, game_id, address, label, added_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(owner_key, game_id, address) DO UPDATE SET label = excluded.label",
        params![
            owner_key,
            favourite.game_id,
            favourite.address,
            favourite.label,
            favourite.added_at
        ],
    )?;
    Ok(())
}

/// Remove `address` for `game_id`; removing one not there changes nothing.
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
        "DELETE FROM favourite_servers WHERE owner_key = ?1 AND game_id = ?2 AND address = ?3",
        params![owner_key, game_id, address],
    )?;
    Ok(())
}

/// Our favourites, the oldest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<FavouriteServer>> {
    let mut stmt = conn.prepare(
        "SELECT game_id, address, label, added_at FROM favourite_servers \
         WHERE owner_key = ?1 ORDER BY added_at, game_id, address",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(FavouriteServer {
            game_id: row.get(0)?,
            address: row.get(1)?,
            label: row.get(2)?,
            added_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
#[path = "favourite_servers_tests.rs"]
mod tests;
