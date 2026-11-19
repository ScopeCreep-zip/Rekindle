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
mod tests {
    use super::*;
    use crate::repo::fixture::{conn, OWNER};

    #[test]
    fn totals_sum_each_game_and_order_by_last_played() {
        let conn = conn();
        record(&conn, OWNER, 4181, 0, 60_000).unwrap();
        record(&conn, OWNER, 4102, 70_000, 100_000).unwrap();
        record(&conn, OWNER, 4181, 200_000, 290_500).unwrap();
        assert_eq!(
            totals(&conn, OWNER).unwrap(),
            vec![
                GameTotal {
                    game_id: 4181,
                    total_secs: 150,
                    last_played: 290_500
                },
                GameTotal {
                    game_id: 4102,
                    total_secs: 30,
                    last_played: 100_000
                },
            ]
        );
        assert!(totals(&conn, "someone-else").unwrap().is_empty());
    }

    #[test]
    fn a_session_ending_before_it_starts_or_with_no_game_is_refused() {
        let conn = conn();
        assert!(record(&conn, OWNER, 4181, 10, 9).is_err());
        assert!(record(&conn, OWNER, 0, 0, 1).is_err());
        assert!(record(&conn, "no-such-identity", 4181, 0, 1).is_err());
    }
}
