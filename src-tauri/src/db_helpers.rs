//! Thin async wrappers around `rekindle_db::Db` that map errors to `String`
//! for IPC.
//!
//! Every DB access in the codebase should go through one of these three
//! helpers — no raw `pool.call()` in business logic.
//!
//! * [`db_call`]  — standard path, propagates errors (commands returning `Result<T, String>`)
//! * [`db_call_or_default`] — graceful degradation (existence checks, counts)
//! * [`db_fire`]  — fire-and-forget writes where failure is non-fatal but logged

use rekindle_db::Db;

/// Standard async DB call — maps `tokio-rusqlite` errors to `String` for IPC.
///
/// Replaces the old 8-line `spawn_blocking` + `lock` + `map_err` + double-`??`
/// pattern used across 80+ call sites.
pub async fn db_call<T, F>(pool: &Db, f: F) -> Result<T, String>
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
pub async fn db_call_or_default<T, F>(pool: &Db, f: F) -> T
where
    T: Send + Default + 'static,
    F: FnOnce(&mut rusqlite::Connection) -> Result<T, rusqlite::Error> + Send + 'static,
{
    pool.call_or_default(f).await
}

/// Fire-and-forget DB operation — queued on the DB thread, logs errors,
/// never blocks the caller. Runs in queue order with every other call on
/// `pool`; no task is spawned (plan C4 `bare-spawn`, `rekindle_db::Db::fire`).
pub fn db_fire<F>(pool: &Db, context: &'static str, f: F)
where
    F: FnOnce(&mut rusqlite::Connection) -> Result<(), rusqlite::Error> + Send + 'static,
{
    pool.fire(context, f);
}
