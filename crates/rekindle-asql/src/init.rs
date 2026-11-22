//! The process's one SQLite initialization (finding DB1b).

use std::sync::Once;

static INITIALIZED: Once = Once::new();

/// Runs SQLite's initialization, SQLCipher's part included, to completion
/// once per process; every other caller waits until it has finished.
///
/// `sqlite3_initialize` sets `isInit` inside its main mutex but runs
/// `SQLITE_EXTRA_INIT` (`sqlcipher_extra_init`) after leaving it, so a
/// concurrent first open returns from its own `sqlite3_initialize` early
/// and its `PRAGMA key` meets `sqlcipher_init == 0`: `sqlcipherCodecAttach`
/// then answers `SQLITE_ERROR` (libsqlite3-sys 0.37 `sqlcipher/sqlite3.c`).
/// rusqlite 0.39 does not serialize that first initialization, and
/// veilid-core's own store (keyvaluedb-sqlite over rusqlite) opens SQLite
/// too. The Host calls this before anything else opens a database
/// (`rekindle_node::host::claim`), and every keyed open (`rekindle_db::open`,
/// `rekindle_vault`'s store) calls it first. Its result is not needed: a
/// failed initialization fails the real open that follows with SQLite's
/// own error.
pub fn init_sqlite() {
    INITIALIZED.call_once(|| drop(rusqlite::Connection::open_in_memory()));
}

/// Whether [`init_sqlite`] has finished in this process.
#[must_use]
pub fn sqlite_initialized() -> bool {
    INITIALIZED.is_completed()
}
