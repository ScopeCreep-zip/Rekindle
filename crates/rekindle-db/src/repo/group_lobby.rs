//! `group_lobbies` and `group_lobby_members` (plan E7.2 G6, SK2-G §5):
//! each group's game lobbies, folded from the lobby kinds (0x0127-0x012B)
//! that members write to their group chains. Every function is one step
//! of the fold and is idempotent: replaying an entry changes nothing and
//! answers `false`. Only the lobby's host (the writer of its `LobbyOpen`)
//! launches or closes it; who may write at all is `rekindle-dm`'s roster
//! admission, applied before the fold.

use rekindle_types::game_server::ServerAddress;
use rusqlite::{params, Connection, OptionalExtension as _};

/// Where a lobby is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyState {
    /// Members may join and leave.
    Open,
    /// The host launched it on its `server`.
    Launched,
    /// The host closed it.
    Closed,
}

impl LobbyState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Launched => "launched",
            Self::Closed => "closed",
        }
    }

    fn from_column(text: &str, index: usize) -> rusqlite::Result<Self> {
        match text {
            "open" => Ok(Self::Open),
            "launched" => Ok(Self::Launched),
            "closed" => Ok(Self::Closed),
            other => Err(rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                format!("lobby state {other:?}").into(),
            )),
        }
    }
}

/// A lobby, with the members who joined it and have not left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyRow {
    /// The host's 16 random bytes, in hex.
    pub lobby_id: String,
    pub host_key: String,
    pub game_id: u32,
    pub state: LobbyState,
    /// The `ServerAddress` text of its launch.
    pub server: Option<String>,
    pub opened_at: i64,
    /// The joined members (never the host), in key order.
    pub members: Vec<String>,
}

/// Fold `host_key`'s `LobbyOpen` of `lobby_id` in `group_id`. A lobby
/// already held keeps its first open (a replay, or another writer reusing
/// the id); a join row held for the host itself (read before the open) is
/// dropped, since the host is never a joined member.
///
/// # Errors
/// The write failed (the group is not held, or a column is refused).
pub fn open(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    lobby_id: &str,
    host_key: &str,
    game_id: u32,
    opened_at: i64,
) -> rusqlite::Result<bool> {
    let inserted = conn.execute(
        "INSERT INTO group_lobbies \
         (owner_key, group_id, lobby_id, host_key, game_id, state, server, opened_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7) \
         ON CONFLICT(owner_key, group_id, lobby_id) DO NOTHING",
        params![
            owner_key,
            group_id,
            lobby_id,
            host_key,
            game_id,
            LobbyState::Open.as_str(),
            opened_at
        ],
    )?;
    if inserted > 0 {
        conn.execute(
            "DELETE FROM group_lobby_members \
             WHERE owner_key = ?1 AND group_id = ?2 AND lobby_id = ?3 AND member_key = ?4",
            params![owner_key, group_id, lobby_id, host_key],
        )?;
    }
    Ok(inserted > 0)
}

/// Fold `member`'s `LobbyJoin`. Refused (`false`) when the lobby is held
/// and is not open or `member` is its host; a join read before the host's
/// open is kept, so the fold does not depend on which chain is read first.
///
/// # Errors
/// The write failed (the group is not held, or a column is refused).
pub fn join(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    lobby_id: &str,
    member: &str,
) -> rusqlite::Result<bool> {
    let inserted = conn.execute(
        "INSERT INTO group_lobby_members (owner_key, group_id, lobby_id, member_key) \
         SELECT ?1, ?2, ?3, ?4 WHERE NOT EXISTS (SELECT 1 FROM group_lobbies \
         WHERE owner_key = ?1 AND group_id = ?2 AND lobby_id = ?3 \
         AND (state != 'open' OR host_key = ?4)) \
         ON CONFLICT(owner_key, group_id, lobby_id, member_key) DO NOTHING",
        params![owner_key, group_id, lobby_id, member],
    )?;
    Ok(inserted > 0)
}

/// Fold `member`'s `LobbyLeave`; leaving a lobby not joined changes nothing.
///
/// # Errors
/// The write failed.
pub fn leave(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    lobby_id: &str,
    member: &str,
) -> rusqlite::Result<bool> {
    let deleted = conn.execute(
        "DELETE FROM group_lobby_members \
         WHERE owner_key = ?1 AND group_id = ?2 AND lobby_id = ?3 AND member_key = ?4",
        params![owner_key, group_id, lobby_id, member],
    )?;
    Ok(deleted > 0)
}

/// Fold `sender`'s `LobbyLaunch` on `server`: applied only from the host,
/// to an open lobby (a replay, a second launch or one after a close changes
/// nothing).
///
/// # Errors
/// The write failed.
pub fn launch(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    lobby_id: &str,
    sender: &str,
    server: &ServerAddress,
) -> rusqlite::Result<bool> {
    let updated = conn.execute(
        "UPDATE group_lobbies SET state = ?5, server = ?6 \
         WHERE owner_key = ?1 AND group_id = ?2 AND lobby_id = ?3 AND host_key = ?4 \
         AND state = 'open'",
        params![
            owner_key,
            group_id,
            lobby_id,
            sender,
            LobbyState::Launched.as_str(),
            server.as_str()
        ],
    )?;
    Ok(updated > 0)
}

/// Fold `sender`'s `LobbyClose`: applied only from the host, to a lobby not
/// yet closed; its server (when launched) is kept.
///
/// # Errors
/// The write failed.
pub fn close(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    lobby_id: &str,
    sender: &str,
) -> rusqlite::Result<bool> {
    let updated = conn.execute(
        "UPDATE group_lobbies SET state = ?5 \
         WHERE owner_key = ?1 AND group_id = ?2 AND lobby_id = ?3 AND host_key = ?4 \
         AND state != 'closed'",
        params![
            owner_key,
            group_id,
            lobby_id,
            sender,
            LobbyState::Closed.as_str()
        ],
    )?;
    Ok(updated > 0)
}

/// The lobby `lobby_id` in `group_id`, with its joined members, or `None`
/// before its open is folded.
///
/// # Errors
/// The query failed, or a row is corrupt.
pub fn get(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    lobby_id: &str,
) -> rusqlite::Result<Option<LobbyRow>> {
    let Some(mut lobby) = conn
        .query_row(
            "SELECT host_key, game_id, state, server, opened_at FROM group_lobbies \
             WHERE owner_key = ?1 AND group_id = ?2 AND lobby_id = ?3",
            params![owner_key, group_id, lobby_id],
            |row| {
                Ok(LobbyRow {
                    lobby_id: lobby_id.to_string(),
                    host_key: row.get(0)?,
                    game_id: row.get(1)?,
                    state: LobbyState::from_column(&row.get::<_, String>(2)?, 2)?,
                    server: row.get(3)?,
                    opened_at: row.get(4)?,
                    members: Vec::new(),
                })
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT member_key FROM group_lobby_members \
         WHERE owner_key = ?1 AND group_id = ?2 AND lobby_id = ?3 ORDER BY member_key",
    )?;
    lobby.members = stmt
        .query_map(params![owner_key, group_id, lobby_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(lobby))
}

#[cfg(test)]
#[path = "group_lobby_tests.rs"]
mod tests;
