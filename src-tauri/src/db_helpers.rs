//! Thin async wrappers around `tokio_rusqlite::Connection::call()`.
//!
//! Every DB access in the codebase should go through one of these three
//! helpers — no raw `pool.call()` in business logic.
//!
//! * [`db_call`]  — standard path, propagates errors (commands returning `Result<T, String>`)
//! * [`db_call_or_default`] — graceful degradation (existence checks, counts)
//! * [`db_fire`]  — fire-and-forget writes where failure is non-fatal but logged

use crate::db::DbPool;

/// Standard async DB call — maps `tokio-rusqlite` errors to `String` for IPC.
///
/// Replaces the old 8-line `spawn_blocking` + `lock` + `map_err` + double-`??`
/// pattern used across 80+ call sites.
pub async fn db_call<T, F>(pool: &DbPool, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut rusqlite::Connection) -> Result<T, rusqlite::Error> + Send + 'static,
{
    pool.call(f).await.map_err(|e| e.to_string())
}

/// Async DB call that returns `T::default()` on *any* failure (query error,
/// connection closed, thread panic).
///
/// Replaces the old `.unwrap_or(Ok(default)).unwrap_or(default)` chains.
pub async fn db_call_or_default<T, F>(pool: &DbPool, f: F) -> T
where
    T: Send + Default + 'static,
    F: FnOnce(&mut rusqlite::Connection) -> Result<T, rusqlite::Error> + Send + 'static,
{
    pool.call(f).await.unwrap_or_default()
}

/// Fire-and-forget DB operation — spawns a task, logs errors, never blocks the caller.
///
/// Replaces the old `tokio::spawn(async { let _ = spawn_blocking(...) })` pattern.
pub fn db_fire<F>(pool: &DbPool, context: &'static str, f: F)
where
    F: FnOnce(&mut rusqlite::Connection) -> Result<(), rusqlite::Error> + Send + 'static,
{
    let pool = pool.clone();
    tokio::spawn(async move {
        if let Err(e) = pool.call(f).await {
            tracing::warn!(context, error = %e, "fire-and-forget DB operation failed");
        }
    });
}
