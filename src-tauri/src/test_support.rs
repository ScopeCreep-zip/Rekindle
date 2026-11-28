//! Shared fixtures for the crate's in-module test suites.
//!
//! `#[cfg(test)]` only — nothing here ships. It exists because
//! `services/search/tests.rs` and `services/cross_device_sync/tests.rs`
//! had each written the same in-memory database fixture, down to the
//! seeded identity row. Two copies of a schema fixture drift the moment
//! `001_init.sql` grows a NOT NULL column: one suite gets updated, the
//! other starts failing for a reason unrelated to what it tests.

use rusqlite::Connection;

/// An in-memory database with the full schema applied and one identity
/// row seeded, which the foreign keys on most tables require. The schema is
/// the shipping one (`rekindle_db::SCHEMA`), never a hand-maintained subset.
#[must_use]
pub fn open_seeded_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(rekindle_db::SCHEMA)
        .expect("apply schema");
    conn.execute(
        "INSERT INTO identity (id, public_key, created_at, friend_request_token) VALUES (1, 'owner_pk', 0, 'aaaaaaaaaaaaaaaaaaaaaaaaaa')",
        [],
    )
    .expect("seed identity");
    conn
}

/// A real offline Host running on a fresh `AppState` (plan GI3): started
/// over the state's own stores (`AppState::host_stores`), so the Host and
/// the shell share one community store and database, and installed in the
/// state's host slot, so every `state_helpers::host_*` path reaches it.
pub struct TestHost {
    _dir: tempfile::TempDir,
    pub state: std::sync::Arc<crate::state::AppState>,
    pub host: std::sync::Arc<rekindle_node::host::Host>,
}

impl TestHost {
    /// Start the Host with no node and set it.
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let state = std::sync::Arc::new(crate::state::AppState::default());
        let host =
            rekindle_node::host::test_fixtures::offline_host_on(&dir, state.host_stores(), None)
                .await;
        state.host.set(std::sync::Arc::clone(&host));
        Self {
            _dir: dir,
            state,
            host,
        }
    }

    /// Stop the Host.
    pub async fn shutdown(self) {
        self.host.shutdown().await.expect("host shuts down");
    }
}
