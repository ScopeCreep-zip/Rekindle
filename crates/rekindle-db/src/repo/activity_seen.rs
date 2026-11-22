//! `activity_seen` (plan E7.15, IV2): what the activity worker last saw of
//! each friend, the fields the feed compares (`rekindle_presence::activity`).
//! The worker writes a friend's row after each fold and reads them all at
//! unlock, so a change it compares against is what it last saw, never the
//! friend row the change already updated. Local only, like `activity_log`;
//! a friend's removal drops their row by cascade.

use std::collections::HashMap;

use rekindle_types::friend::GameInfoState;
use rusqlite::{params, Connection, Row};

/// The fields of a friend the feed compares, as the worker last saw them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenFriend {
    /// The friendship was accepted.
    pub accepted: bool,
    pub display_name: String,
    pub status_message: Option<String>,
    pub bio: Option<String>,
    /// Their current game; `None` when they played nothing or did not share it.
    pub game: Option<GameInfoState>,
}

/// Every friend the worker has seen, by public key (hex).
///
/// # Errors
/// The query failed, or a stored game does not parse.
pub fn load(conn: &Connection, owner_key: &str) -> rusqlite::Result<HashMap<String, SeenFriend>> {
    let mut stmt = conn.prepare(
        "SELECT friend_key, accepted, display_name, status_message, bio, game_json \
         FROM activity_seen WHERE owner_key = ?1",
    )?;
    let rows = stmt.query_map(params![owner_key], read_row)?;
    rows.collect()
}

/// Store what the worker now holds of `friend_key`, replacing what it held.
///
/// # Errors
/// The write failed, `friend_key` is not a friend row of ours, or the game
/// would not serialize.
pub fn save(
    conn: &Connection,
    owner_key: &str,
    friend_key: &str,
    seen: &SeenFriend,
) -> rusqlite::Result<()> {
    let game = seen
        .game
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT INTO activity_seen \
         (owner_key, friend_key, accepted, display_name, status_message, bio, game_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(owner_key, friend_key) DO UPDATE SET accepted = excluded.accepted, \
         display_name = excluded.display_name, status_message = excluded.status_message, \
         bio = excluded.bio, game_json = excluded.game_json",
        params![
            owner_key,
            friend_key,
            seen.accepted,
            seen.display_name,
            seen.status_message,
            seen.bio,
            game
        ],
    )?;
    Ok(())
}

/// Forget `friend_key` (no longer a friend: removed or blocked).
///
/// # Errors
/// The write failed.
pub fn forget(conn: &Connection, owner_key: &str, friend_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM activity_seen WHERE owner_key = ?1 AND friend_key = ?2",
        params![owner_key, friend_key],
    )?;
    Ok(())
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<(String, SeenFriend)> {
    let game: Option<String> = row.get(5)?;
    let game = game
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;
    Ok((
        row.get(0)?,
        SeenFriend {
            accepted: row.get(1)?,
            display_name: row.get(2)?,
            status_message: row.get(3)?,
            bio: row.get(4)?,
            game,
        },
    ))
}
