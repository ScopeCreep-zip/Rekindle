use rusqlite::Connection;

/// The owner every fixture row belongs to.
pub const OWNER: &str = "owner-pk";

/// An in-memory database with the schema and [`OWNER`]'s identity row,
/// which the per-owner tables reference.
pub fn conn() -> Connection {
    crate::test_support::schema_conn(OWNER)
}

/// The database file at `path` with the schema and [`OWNER`]'s
/// identity row, made on the first open and kept by a later one (a
/// restart).
pub fn conn_at(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    conn.execute_batch(crate::SCHEMA).unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO identity (public_key, display_name, created_at, \
             friend_request_token) VALUES (?1, 'me', 0, 'aaaaaaaaaaaaaaaaaaaaaaaaaa')",
        [OWNER],
    )
    .unwrap();
    conn
}

/// An in-memory database with the schema and an identity row for the
/// key `owner` (the ratchet tables key their owner by its hex).
pub fn identity_conn(owner: &[u8; 32]) -> Connection {
    crate::test_support::schema_conn(&super::identity_text(owner))
}

/// A `communities` row for [`OWNER`], which the per-community tables
/// reference.
pub fn community(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO communities (owner_key, id, name, joined_at) VALUES (?1, ?2, ?2, 0)",
        [OWNER, id],
    )
    .unwrap();
}
