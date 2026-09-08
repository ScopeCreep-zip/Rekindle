//! Shared fixtures for the crate's in-module test suites.
//!
//! `#[cfg(test)]` only — nothing here ships. It exists because
//! `services/search/tests.rs` and `services/cross_device_sync/tests.rs`
//! had each written the same in-memory database fixture, down to the
//! seeded identity row. Two copies of a schema fixture drift the moment
//! `001_init.sql` grows a NOT NULL column: one suite gets updated, the
//! other starts failing for a reason unrelated to what it tests.

use rusqlite::Connection;

/// The live schema. `SCHEMA_VERSION` in `db.rs` is bumped whenever this
/// file changes, so tests always run against the shipping schema rather
/// than a hand-maintained subset.
const MIGRATION: &str = include_str!("../migrations/001_init.sql");

/// An in-memory database with the full schema applied and one identity
/// row seeded, which the foreign keys on most tables require.
#[must_use]
pub fn open_seeded_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(MIGRATION).expect("apply migration");
    conn.execute(
        "INSERT INTO identity (id, public_key, created_at) VALUES (1, 'owner_pk', 0)",
        [],
    )
    .expect("seed identity");
    conn
}
