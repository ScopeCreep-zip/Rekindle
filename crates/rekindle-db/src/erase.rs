//! Erased key material leaves the `-wal` file too (threat model I14; RV5
//! review M3, M4, L2, L3).
//!
//! `secure_delete` zeroes freed pages in the database file, but a WAL
//! keeps every committed page image until a checkpoint copies it back and
//! the file is truncated. So every change that erases key material (a
//! deleted session, prekey secret, sender key, group chain or presence
//! key, and an eviction inside a record blob: a ratchet step's spent chain
//! and skipped keys, the archived-session cap, a delete-then-insert) marks
//! this connection [`erased`], and the [`crate::Db`] wrapper, once the call
//! that marked it returns outside any transaction (its outermost commit
//! done), queues a `wal_checkpoint(TRUNCATE)` behind the calls already
//! waiting. The checkpoint never runs inside the call that erased, so that
//! call's answer is never held behind it (M4), and SQLite's auto-checkpoint
//! (1000 pages, set in `open`) bounds what it copies.
//!
//! The mark is per connection, which is per database file: one identity
//! per file (L3). It lives on the connection's own thread (`rekindle-asql`
//! runs each connection on one), so no other connection's change can hide
//! or take it. A rolled-back erase leaves it set: one spare checkpoint.
//!
//! Key-material rows are deleted only through [`delete`] (`cargo xtask
//! check-key-erasure`).

use std::cell::Cell;

use rusqlite::Connection;

thread_local! {
    static ERASED: Cell<bool> = const { Cell::new(false) };
}

/// The tables holding key material, which `cargo xtask check-key-erasure`
/// keeps to [`delete`].
pub const KEY_MATERIAL_TABLES: [&str; 5] = [
    "ratchet_sessions",
    "signal_prekeys",
    "group_dm_sender_keys",
    "group_dm_chains",
    "presence_keys",
];

/// Mark that the change running on this connection erased key material.
pub(crate) fn erased() {
    ERASED.with(|e| e.set(true));
}

/// Delete key material with `sql`; marks [`erased`] when a row went.
///
/// # Errors
/// The delete failed.
pub(crate) fn delete(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<usize> {
    let deleted = conn.execute(sql, params)?;
    if deleted > 0 {
        erased();
    }
    Ok(deleted)
}

/// Whether an outermost commit on `conn` since the last take erased key
/// material: taken (cleared) only outside any transaction, so an erase
/// nested in a caller's transaction counts at that caller's commit.
pub(crate) fn take_committed(conn: &Connection) -> bool {
    conn.is_autocommit() && ERASED.with(|e| e.replace(false))
}

/// Checkpoint the WAL into the database and truncate it to zero bytes. The
/// erase has committed, so a failure (a reader holding the WAL) is logged,
/// not returned: the next truncating checkpoint, or the connection's
/// close, removes the frames.
pub(crate) fn truncate_wal(conn: &Connection) {
    match conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(0) => {}
        Ok(_) => tracing::warn!("WAL checkpoint blocked by a reader; erased frames remain"),
        Err(e) => tracing::warn!(error = %e, "WAL checkpoint failed; erased frames remain"),
    }
}

#[cfg(test)]
#[path = "erase_tests.rs"]
mod tests;
