//! The `missed_calls` log (plan E4.1 S1): a 1:1 call whose ring expired
//! unanswered, on either side. Moved here from the desktop's call adapter
//! (`src-tauri/src/services/calls_adapter/deps_impl.rs`, both ring
//! timeouts) and `src-tauri/src/services/call_runtime.rs`
//! (`get_missed_calls_inner`), so the Host's call adapter (E4.1 S7) writes
//! and reads the same rows through one place.

use rusqlite::{params, Connection};

/// How many missed calls one read returns, newest first.
pub const MISSED_CALLS_LIMIT: i64 = 200;

/// One missed call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissedCall {
    pub call_id: String,
    /// The peer's identity key (hex).
    pub peer_key: String,
    /// `CallKind::as_u8`: 0 audio, 1 video.
    pub kind: u8,
    /// When the ring expired, ms since the epoch.
    pub expired_at: i64,
}

/// Record that call `call_id` with `peer_key` rang out at `expired_at_ms`.
/// A second record of the same call is ignored.
///
/// # Errors
/// The write failed.
pub fn record_missed(
    conn: &Connection,
    owner_key: &str,
    call_id: &str,
    peer_key: &str,
    kind: u8,
    expired_at_ms: u64,
) -> rusqlite::Result<()> {
    let expired = i64::try_from(expired_at_ms).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT OR IGNORE INTO missed_calls \
         (call_id, owner_key, peer_key, kind, expired_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![call_id, owner_key, peer_key, i64::from(kind), expired],
    )?;
    Ok(())
}

/// `owner_key`'s latest [`MISSED_CALLS_LIMIT`] missed calls, newest first.
///
/// # Errors
/// The read failed.
pub fn list_missed(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<MissedCall>> {
    let mut stmt = conn.prepare(
        "SELECT call_id, peer_key, kind, expired_at FROM missed_calls \
         WHERE owner_key = ?1 ORDER BY expired_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![owner_key, MISSED_CALLS_LIMIT], |row| {
        Ok(MissedCall {
            call_id: row.get(0)?,
            peer_key: row.get(1)?,
            kind: row.get(2)?,
            expired_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
