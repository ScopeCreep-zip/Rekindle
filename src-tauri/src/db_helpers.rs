//! Thin async wrappers around `rekindle_db::Db` that map errors to `String`
//! for IPC.
//!
//! Every DB access in the codebase should go through one of these two
//! helpers — no raw `pool.call()` in business logic. A failed read is an
//! error the caller handles, never a silent default (plan D2).
//!
//! * [`db_call`]  — standard path, propagates errors (commands returning `Result<T, String>`)
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

/// Fire-and-forget DB operation — queued on the DB thread, logs errors,
/// never blocks the caller. Runs in queue order with every other call on
/// `pool`; no task is spawned (plan C4 `bare-spawn`, `rekindle_db::Db::fire`).
pub fn db_fire<F>(pool: &Db, context: &'static str, f: F)
where
    F: FnOnce(&mut rusqlite::Connection) -> Result<(), rusqlite::Error> + Send + 'static,
{
    pool.fire(context, f);
}
