//! `game_sessions` (plan E7.2): the time spent in each detected game. The
//! Host's one game publisher records a session when it ends; the profile
//! reads the totals per game and when each was last played.

use rusqlite::{params, Connection};

pub use rekindle_types::dto::presence::GameTotal;

/// Record a session of `game_id` from `started_at` to `ended_at` (Unix ms).
///
/// # Errors
/// The write failed, or the row breaks the table's checks (game id 0, an
/// end before the start).
pub fn record(
    conn: &Connection,
    owner_key: &str,
    game_id: u32,
    started_at: i64,
    ended_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO game_sessions (owner_key, game_id, started_at, ended_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![owner_key, game_id, started_at, ended_at],
    )?;
    Ok(())
}

/// Every game played, the most recently played first.
///
/// # Errors
/// The query failed.
pub fn totals(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<GameTotal>> {
    let mut stmt = conn.prepare(
        "SELECT game_id, SUM(ended_at - started_at) / 1000, MAX(ended_at) \
         FROM game_sessions WHERE owner_key = ?1 \
         GROUP BY game_id ORDER BY MAX(ended_at) DESC, game_id",
    )?;
    let rows = stmt.query_map([owner_key], |row| {
        Ok(GameTotal {
            game_id: row.get(0)?,
            total_secs: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            last_played: row.get(2)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
#[path = "game_sessions_tests.rs"]
mod tests;
