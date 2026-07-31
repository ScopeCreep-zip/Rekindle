use std::fmt;
use std::sync::Arc;

use rusqlite::Connection;

/// Error type for async DB operations, replacing `tokio_rusqlite::Error`.
///
/// Two failure modes: the closure returned an application error, or the
/// background task panicked / was cancelled.
#[derive(Debug)]
pub enum DbError<E> {
    /// The closure returned `Err(e)`.
    Rusqlite(E),
    /// `spawn_blocking` panicked or was cancelled.
    Internal(String),
}

impl<E: fmt::Display> fmt::Display for DbError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rusqlite(e) => write!(f, "{e}"),
            Self::Internal(msg) => write!(f, "internal db error: {msg}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for DbError<E> {}

impl<E> From<E> for DbError<E> {
    fn from(e: E) -> Self {
        Self::Rusqlite(e)
    }
}

/// Async database handle backed by `tokio::task::spawn_blocking`.
///
/// Wraps a single `rusqlite::Connection` behind `Arc<std::sync::Mutex>` and
/// exposes an async `call()` API identical to the former `tokio_rusqlite`
/// dependency. Clone + Send + Sync, so `tauri::State<'_, DbPool>` works.
#[derive(Clone)]
pub struct DbPool {
    conn: Arc<std::sync::Mutex<Connection>>,
}

impl DbPool {
    /// Wrap a synchronous connection into an async pool handle.
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(std::sync::Mutex::new(conn)),
        }
    }

    /// Run a closure on the connection via `spawn_blocking`.
    ///
    /// The closure receives `&mut Connection` (matching the old
    /// `tokio_rusqlite` signature).
    pub async fn call<F, T>(&self, f: F) -> Result<T, DbError<rusqlite::Error>>
    where
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let mut guard = conn
                .lock()
                .map_err(|e| DbError::Internal(format!("mutex poisoned: {e}")))?;
            f(&mut guard).map_err(DbError::Rusqlite)
        })
        .await
        .map_err(|e| DbError::Internal(format!("spawn_blocking join: {e}")))?
    }
}

/// Bump this every time `001_init.sql` changes.  On mismatch the entire
/// database is wiped and recreated from the schema -- safe because the app
/// is not live yet and identity keys live in Stronghold, not `SQLite`.
const SCHEMA_VERSION: i64 = 66;

/// Result of opening the database -- includes a flag indicating whether the
/// schema was recreated from scratch (so the caller can wipe dependent storage).
pub struct DbOpenResult {
    pub pool: DbPool,
    /// `true` when the schema version changed and all tables were dropped and
    /// recreated.  The caller should wipe Stronghold files and Veilid storage
    /// to avoid orphaned state.
    pub schema_reset: bool,
}

/// Open (or create) a `SQLite` database at `db_path` and run the initial schema
/// migration.  Returns a `DbOpenResult` with the pool and a reset flag.
///
/// The raw `rusqlite::Connection` is created and configured synchronously
/// (PRAGMAs, schema check), then wrapped in `DbPool` which uses
/// `spawn_blocking` for all future DB access.
pub fn create_pool(db_path: &str) -> Result<DbOpenResult, String> {
    let conn =
        Connection::open(db_path).map_err(|e| format!("failed to connect to database: {e}"))?;

    // Enable WAL mode for better concurrent-read performance.
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("failed to set WAL mode: {e}"))?;

    // Enable foreign key constraint enforcement (off by default in SQLite).
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to enable foreign keys: {e}"))?;

    // Check schema version -- wipe and recreate if stale.
    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);

    let schema_reset = current != SCHEMA_VERSION;

    if schema_reset {
        if current != 0 {
            tracing::info!(
                old = current,
                new = SCHEMA_VERSION,
                "schema version mismatch -- recreating database"
            );
        }
        drop_all_tables(&conn)?;
        conn.execute_batch(include_str!("../migrations/001_init.sql"))
            .map_err(|e| format!("failed to run schema: {e}"))?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| format!("failed to set schema version: {e}"))?;
    }

    // Wrap configured connection.
    Ok(DbOpenResult {
        pool: DbPool::new(conn),
        schema_reset,
    })
}

/// Drop every user table so the schema can be cleanly re-applied.
///
/// Virtual tables (FTS5) must be dropped before regular tables -- they own
/// shadow tables (`<name>_data`, `<name>_idx`, etc.) which SQLite refuses
/// to drop directly. We identify them via `sql LIKE 'CREATE VIRTUAL%'`
/// and skip rows whose `sql` is NULL (shadow tables).
fn drop_all_tables(conn: &Connection) -> Result<(), String> {
    // Must disable FK checks while dropping to avoid ordering issues.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| format!("failed to disable foreign keys: {e}"))?;

    let virtual_tables = list_tables_matching(
        conn,
        "type='table' AND sql LIKE 'CREATE VIRTUAL%' AND name NOT LIKE 'sqlite_%'",
    )?;
    for table in &virtual_tables {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))
            .map_err(|e| format!("failed to drop virtual table {table}: {e}"))?;
    }

    let real_tables = list_tables_matching(
        conn,
        "type='table' AND sql LIKE 'CREATE TABLE%' AND name NOT LIKE 'sqlite_%'",
    )?;
    for table in &real_tables {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))
            .map_err(|e| format!("failed to drop table {table}: {e}"))?;
    }

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to re-enable foreign keys: {e}"))?;

    Ok(())
}

fn list_tables_matching(conn: &Connection, where_clause: &str) -> Result<Vec<String>, String> {
    let sql = format!("SELECT name FROM sqlite_master WHERE {where_clause}");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("failed to list tables: {e}"))?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("failed to query tables: {e}"))?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(names)
}

/// Extract a `String` column by name, returning `""` on any failure.
pub fn get_str(row: &rusqlite::Row<'_>, col: &str) -> String {
    row.get::<_, String>(col).unwrap_or_default()
}

/// Extract an optional `String` column by name.
pub fn get_str_opt(row: &rusqlite::Row<'_>, col: &str) -> Option<String> {
    row.get::<_, Option<String>>(col).ok().flatten()
}

/// Extract an `i64` column by name, returning `0` on any failure.
pub fn get_i64(row: &rusqlite::Row<'_>, col: &str) -> i64 {
    row.get::<_, i64>(col).unwrap_or_default()
}

/// Current UNIX timestamp in milliseconds.
pub fn timestamp_now() -> i64 {
    rekindle_utils::timestamp_ms_i64()
}
