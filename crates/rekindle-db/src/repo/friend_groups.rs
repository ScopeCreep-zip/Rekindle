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
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_deleted_group_leaves_its_friends_ungrouped() {
        let conn = fixture::conn();
        let id = create(&conn, OWNER, "Raid").unwrap();
        assert!(rename(&conn, OWNER, id, "Clan").unwrap());
        assert_eq!(name(&conn, OWNER, id).unwrap().as_deref(), Some("Clan"));
        crate::repo::friends::insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
        crate::repo::friends::set(
            &conn,
            OWNER,
            "bob",
            crate::repo::friends::Column::GroupId,
            id,
        )
        .unwrap();
        assert!(delete(&conn, OWNER, id).unwrap());
        assert!(!rename(&conn, OWNER, id, "Gone").unwrap());
        let row = &crate::repo::friends::load_all(&conn, OWNER).unwrap()[0];
        assert_eq!(row.group_name, None);
    }
}
