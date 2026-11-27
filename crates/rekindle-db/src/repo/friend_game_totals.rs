//! `friend_game_totals` (plan E7.2): a friend's game totals as their last
//! `GameTotals` DmFrame (0x0172) stated them. A frame replaces them all;
//! the friend row's removal (unfriend, block) removes them by cascade.

use rusqlite::{params, Connection};

use super::game_sessions::GameTotal;

/// Replace `friend_key`'s totals with `totals`.
///
/// # Errors
/// The write failed, or a row breaks the table's checks (game id 0, no
/// such friend).
pub fn replace(
    conn: &mut Connection,
    owner_key: &str,
    friend_key: &str,
    totals: &[GameTotal],
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM friend_game_totals WHERE owner_key = ?1 AND friend_key = ?2",
        params![owner_key, friend_key],
    )?;
    for t in totals {
        tx.execute(
            "INSERT INTO friend_game_totals \
             (owner_key, friend_key, game_id, total_secs, last_played) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                owner_key,
                friend_key,
                t.game_id,
                i64::try_from(t.total_secs).unwrap_or(i64::MAX),
                t.last_played
            ],
        )?;
    }
    tx.commit()
}

/// The game ids of the totals `friend_key` shares; `None` while we hold
/// none from them (the activity feed's "first time" has nothing to
/// compare against).
///
/// # Errors
/// The query failed.
pub fn game_ids(
    conn: &Connection,
    owner_key: &str,
    friend_key: &str,
) -> rusqlite::Result<Option<std::collections::BTreeSet<u32>>> {
    let totals = of(conn, owner_key, friend_key)?;
    Ok((!totals.is_empty()).then(|| totals.iter().map(|t| t.game_id).collect()))
}

/// `friend_key`'s totals, the most recently played first.
///
/// # Errors
/// The query failed.
pub fn of(
    conn: &Connection,
    owner_key: &str,
    friend_key: &str,
) -> rusqlite::Result<Vec<GameTotal>> {
    let mut stmt = conn.prepare(
        "SELECT game_id, total_secs, last_played FROM friend_game_totals \
         WHERE owner_key = ?1 AND friend_key = ?2 ORDER BY last_played DESC, game_id",
    )?;
    let rows = stmt.query_map(params![owner_key, friend_key], |row| {
        Ok(GameTotal {
            game_id: row.get(0)?,
            total_secs: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            last_played: row.get(2)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
#[path = "friend_game_totals_tests.rs"]
mod tests;
