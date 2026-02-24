use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;

/// Acquire the server DB lock with poison recovery.
///
/// The 4-line lock boilerplate is repeated 70+ times across rpc.rs,
/// community_host.rs, etc. This helper centralizes it.
pub fn lock_db(db: &Arc<Mutex<Connection>>) -> MutexGuard<'_, Connection> {
    db.lock().unwrap_or_else(|e| {
        tracing::error!(error = %e, "server db mutex poisoned — recovering");
        e.into_inner()
    })
}

/// Execute a synchronous DB operation, mapping rusqlite errors to `String`.
pub fn db_call<T, F>(db: &Arc<Mutex<Connection>>, f: F) -> Result<T, String>
where
    F: FnOnce(&Connection) -> Result<T, rusqlite::Error>,
{
    let conn = lock_db(db);
    f(&conn).map_err(|e| e.to_string())
}

/// Execute a synchronous DB operation, returning `T::default()` on failure.
pub fn db_call_or_default<T: Default, F>(db: &Arc<Mutex<Connection>>, f: F) -> T
where
    F: FnOnce(&Connection) -> Result<T, rusqlite::Error>,
{
    let conn = lock_db(db);
    match f(&conn) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "server db query failed — returning default");
            T::default()
        }
    }
}

/// Fire-and-forget: log errors, never propagate.
pub fn db_fire<F>(db: &Arc<Mutex<Connection>>, context: &str, f: F)
where
    F: FnOnce(&Connection) -> Result<(), rusqlite::Error>,
{
    let conn = lock_db(db);
    if let Err(e) = f(&conn) {
        tracing::error!(error = %e, context = context, "server db fire-and-forget failed");
    }
}
