//! `friend_groups`: the buddy list's named groups (plan E2.1). A deleted
//! group's friends fall back to no group (`friends.group_id ... ON DELETE
//! SET NULL`).

use rusqlite::{params, Connection, OptionalExtension as _};

/// Create a group; its id.
///
/// # Errors
/// The write failed (a name the owner already uses is a constraint error).
pub fn create(conn: &Connection, owner_key: &str, name: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO friend_groups (owner_key, name) VALUES (?1, ?2)",
        params![owner_key, name],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Rename the owner's group `id`; whether it exists.
///
/// # Errors
/// The write failed.
pub fn rename(conn: &Connection, owner_key: &str, id: i64, name: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE friend_groups SET name = ?1 WHERE owner_key = ?2 AND id = ?3",
        params![name, owner_key, id],
    )? > 0)
}

/// Delete the owner's group `id`; whether it existed.
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner_key: &str, id: i64) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM friend_groups WHERE owner_key = ?1 AND id = ?2",
        params![owner_key, id],
    )? > 0)
}

/// The name of the owner's group `id`.
///
/// # Errors
/// The query failed.
pub fn name(conn: &Connection, owner_key: &str, id: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT name FROM friend_groups WHERE owner_key = ?1 AND id = ?2",
        params![owner_key, id],
        |row| row.get(0),
    )
    .optional()
}

#[cfg(test)]
#[path = "friend_groups_tests.rs"]
mod tests;
