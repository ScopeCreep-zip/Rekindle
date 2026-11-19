//! `hidden_games` (plan E7.2, prod research batch 7): the games never shown
//! to anyone. A hidden game is still recorded in `game_sessions` for the
//! local history; it is never the current game friends see and never in
//! the totals sent to them.

use rusqlite::{params, Connection};

/// Hide `game_id`; hiding a hidden game changes nothing.
///
/// # Errors
/// The write failed, or `game_id` is 0 (the table's check).
pub fn hide(conn: &Connection, owner_key: &str, game_id: u32) -> rusqlite::Result<()> {
    conn.execute(
        // ON CONFLICT, not OR IGNORE: OR IGNORE also skips the CHECK.
        "INSERT INTO hidden_games (owner_key, game_id) VALUES (?1, ?2) \
         ON CONFLICT(owner_key, game_id) DO NOTHING",
        params![owner_key, game_id],
    )?;
    Ok(())
}

/// Show `game_id` again; showing a game not hidden changes nothing.
///
/// # Errors
/// The write failed.
pub fn unhide(conn: &Connection, owner_key: &str, game_id: u32) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM hidden_games WHERE owner_key = ?1 AND game_id = ?2",
        params![owner_key, game_id],
    )?;
    Ok(())
}

/// The hidden game ids, ascending.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<u32>> {
    let mut stmt =
        conn.prepare("SELECT game_id FROM hidden_games WHERE owner_key = ?1 ORDER BY game_id")?;
    let rows = stmt.query_map(params![owner_key], |row| row.get(0))?;
    rows.collect()
}

/// Whether `game_id` is hidden.
///
/// # Errors
/// The query failed.
pub fn is_hidden(conn: &Connection, owner_key: &str, game_id: u32) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM hidden_games WHERE owner_key = ?1 AND game_id = ?2)",
        params![owner_key, game_id],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn hide_and_unhide_are_idempotent_and_listed() {
        let conn = fixture::conn();
        assert!(list(&conn, OWNER).unwrap().is_empty());
        hide(&conn, OWNER, 4181).unwrap();
        hide(&conn, OWNER, 4181).unwrap();
        hide(&conn, OWNER, 12).unwrap();
        assert_eq!(list(&conn, OWNER).unwrap(), vec![12, 4181]);
        assert!(is_hidden(&conn, OWNER, 4181).unwrap());
        unhide(&conn, OWNER, 4181).unwrap();
        unhide(&conn, OWNER, 4181).unwrap();
        assert_eq!(list(&conn, OWNER).unwrap(), vec![12]);
        assert!(!is_hidden(&conn, OWNER, 4181).unwrap());
        assert!(hide(&conn, OWNER, 0).is_err(), "game id 0 is no game");
    }
}
