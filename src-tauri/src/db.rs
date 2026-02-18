use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// Thread-safe database handle wrapping a single `rusqlite::Connection`.
///
/// `rusqlite::Connection` is `Send` but not `Sync`, so we guard it with
/// `std::sync::Mutex` (not `parking_lot` — its guards are `!Send` which can
/// cause issues with async runtimes).
pub type DbPool = Arc<Mutex<Connection>>;

/// Bump this every time `001_init.sql` changes.  On mismatch the entire
/// database is wiped and recreated from the schema — safe because the app
/// is not live yet and identity keys live in Stronghold, not `SQLite`.
const SCHEMA_VERSION: i64 = 15;

/// Result of opening the database — includes a flag indicating whether the
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
pub fn create_pool(db_path: &str) -> Result<DbOpenResult, String> {
    let conn = Connection::open(db_path)
        .map_err(|e| format!("failed to connect to database: {e}"))?;

    // Enable WAL mode for better concurrent-read performance.
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("failed to set WAL mode: {e}"))?;

    // Enable foreign key constraint enforcement (off by default in SQLite).
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to enable foreign keys: {e}"))?;

    // Check schema version — wipe and recreate if stale.
    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);

    let schema_reset = current != SCHEMA_VERSION;

    if schema_reset {
        if current != 0 {
            tracing::info!(
                old = current,
                new = SCHEMA_VERSION,
                "schema version mismatch — recreating database"
            );
        }
        drop_all_tables(&conn)?;
        conn.execute_batch(include_str!("../migrations/001_init.sql"))
            .map_err(|e| format!("failed to run schema: {e}"))?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| format!("failed to set schema version: {e}"))?;
    }

    Ok(DbOpenResult {
        pool: Arc::new(Mutex::new(conn)),
        schema_reset,
    })
}

/// Drop every user table so the schema can be cleanly re-applied.
fn drop_all_tables(conn: &Connection) -> Result<(), String> {
    // Must disable FK checks while dropping to avoid ordering issues.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| format!("failed to disable foreign keys: {e}"))?;

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| format!("failed to list tables: {e}"))?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("failed to query tables: {e}"))?
        .filter_map(std::result::Result::ok)
        .collect();
    drop(stmt);

    for table in &tables {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))
            .map_err(|e| format!("failed to drop table {table}: {e}"))?;
    }

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to re-enable foreign keys: {e}"))?;

    Ok(())
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
