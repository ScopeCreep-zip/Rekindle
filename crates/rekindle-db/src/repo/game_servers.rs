//! A community's game servers (plan E3.1 H1): the entries its members
//! added, as each peer accepted them off the mesh.

use rekindle_types::game_server::GameServerInfo;
use rusqlite::{params, Connection, OptionalExtension};

/// Store `server`; an id already stored keeps its first entry, so a replay
/// cannot rewrite it.
///
/// # Errors
/// The insert failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    server: &GameServerInfo,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO game_servers \
         (owner_key, community_id, id, game_id, label, address, added_by, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT (owner_key, community_id, id) DO NOTHING",
        params![
            owner_key,
            community_id,
            server.id,
            server.game_id,
            server.label,
            server.address,
            server.added_by,
            server.created_at.cast_signed(),
        ],
    )?;
    Ok(())
}

/// Drop the server `id`.
///
/// # Errors
/// The delete failed.
pub fn remove(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM game_servers WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
        params![owner_key, community_id, id],
    )?;
    Ok(())
}

/// The server `id`'s label, when it is stored.
///
/// # Errors
/// The query failed.
pub fn label(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT label FROM game_servers WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
        params![owner_key, community_id, id],
        |row| row.get(0),
    )
    .optional()
}

/// The community's servers, newest first, ties by id.
///
/// # Errors
/// The query failed.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<GameServerInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, game_id, label, address, added_by, created_at FROM game_servers \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY created_at DESC, id ASC",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id], |row| {
        Ok(GameServerInfo {
            id: row.get(0)?,
            game_id: row.get(1)?,
            label: row.get(2)?,
            address: row.get(3)?,
            added_by: row.get(4)?,
            created_at: row.get::<_, i64>(5)?.cast_unsigned(),
        })
    })?;
    rows.collect()
}
