//! Repositories: every query on the tables more than one host reads or
//! writes (plan C5.3, `steps-10-19.md` N1).
//!
//! Each function takes the connection and runs on the database thread,
//! inside a [`crate::Db::call`] closure, so several can share one
//! transaction (`rusqlite::Transaction` derefs to the connection). This is
//! the shape matrix-sdk-sqlite gives its table helpers
//! (`SqliteKeyValueStoreConnExt`, `SqliteTransactionExt` over
//! `rusqlite::Connection`). The SQL lives here and nowhere else.

pub mod audit;
pub mod communities;
pub mod dht_outbox;
pub mod friends;
pub mod governance_cache;
pub mod identity;
pub mod members;
pub mod pending_requests;
mod role_ids;

#[cfg(test)]
pub(crate) mod fixture {
    use rusqlite::Connection;

    /// The owner every fixture row belongs to.
    pub const OWNER: &str = "owner-pk";

    /// An in-memory database with the schema and [`OWNER`]'s identity row,
    /// which the per-owner tables reference.
    pub fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(crate::SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO identity (public_key, display_name, created_at) VALUES (?1, 'me', 0)",
            [OWNER],
        )
        .unwrap();
        conn
    }
}
