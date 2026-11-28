//! `transfers` (plan E7.2 G12, SK2-G design §4): the Downloads view's rows
//! and each one's resume checkpoint. The Host's transfer driver is the
//! only writer; the skin capabilities (`StartDownload`, `StopDownload`,
//! `ExecuteDownload`) read the state from here.

use rekindle_types::dto::transfer::{TransferRow, TransferState, TransferTarget};
use rusqlite::{params, Connection, OptionalExtension};

const COLUMNS: &str = "attachment_id, target_json, state, chunks_done, chunk_count, bytes_done, \
                       total, dest_name, dest_path, started_at, finished_at";

fn bad(text: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, text.into())
}

/// A stored byte count (the table's checks keep it non-negative).
fn count(n: i64) -> rusqlite::Result<u64> {
    u64::try_from(n).map_err(|_| bad(format!("byte count {n}")))
}

/// A byte count as SQLite's integer.
fn stored(n: u64) -> rusqlite::Result<i64> {
    i64::try_from(n)
        .map_err(|_| rusqlite::Error::ToSqlConversionFailure(format!("byte count {n}").into()))
}

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferRow> {
    let target: String = crate::row::get(row, "target_json")?;
    let target: TransferTarget =
        serde_json::from_str(&target).map_err(|e| bad(format!("transfer target: {e}")))?;
    let state: String = crate::row::get(row, "state")?;
    let state = TransferState::parse(&state).ok_or_else(|| bad(format!("state {state}")))?;
    Ok(TransferRow {
        attachment_id: crate::row::get(row, "attachment_id")?,
        target,
        state,
        chunks_done: crate::row::get(row, "chunks_done")?,
        chunk_count: crate::row::get(row, "chunk_count")?,
        bytes_done: count(crate::row::get(row, "bytes_done")?)?,
        total: count(crate::row::get(row, "total")?)?,
        dest_name: crate::row::get(row, "dest_name")?,
        dest_path: crate::row::get(row, "dest_path")?,
        started_at: crate::row::get(row, "started_at")?,
        finished_at: crate::row::get(row, "finished_at")?,
    })
}

/// Begin (or restart) the download of `row.attachment_id` as `row.state`.
/// A row already present keeps its checkpoint and `started_at`; its
/// target and destination are replaced.
///
/// # Errors
/// The write failed.
pub fn begin(conn: &Connection, owner_key: &str, row: &TransferRow) -> rusqlite::Result<()> {
    let target = serde_json::to_string(&row.target).map_err(|e| bad(e.to_string()))?;
    conn.execute(
        "INSERT INTO transfers (owner_key, attachment_id, target_json, direction, state, \
         dest_name, dest_path, started_at) VALUES (?1, ?2, ?3, 'download', ?4, ?5, ?6, ?7) \
         ON CONFLICT(owner_key, attachment_id) DO UPDATE SET target_json = excluded.target_json, \
         state = excluded.state, dest_name = excluded.dest_name, \
         dest_path = excluded.dest_path, finished_at = NULL",
        params![
            owner_key,
            row.attachment_id,
            target,
            row.state.as_str(),
            row.dest_name,
            row.dest_path,
            row.started_at
        ],
    )?;
    Ok(())
}

/// Record the checkpoint: `chunks_done` of `chunk_count` held,
/// `bytes_done` of `total`.
///
/// # Errors
/// The write failed.
pub fn checkpoint(
    conn: &Connection,
    owner_key: &str,
    attachment_id: &str,
    (chunks_done, chunk_count): (u32, u32),
    (bytes_done, total): (u64, u64),
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE transfers SET chunks_done = ?3, chunk_count = ?4, bytes_done = ?5, total = ?6 \
         WHERE owner_key = ?1 AND attachment_id = ?2",
        params![
            owner_key,
            attachment_id,
            chunks_done,
            chunk_count,
            stored(bytes_done)?,
            stored(total)?
        ],
    )?;
    Ok(())
}

/// Set the state; `finished_at` is stamped for `Done` and cleared
/// otherwise. `false` when there is no such transfer.
///
/// # Errors
/// The write failed.
pub fn set_state(
    conn: &Connection,
    owner_key: &str,
    attachment_id: &str,
    state: TransferState,
    now: i64,
) -> rusqlite::Result<bool> {
    let finished = (state == TransferState::Done).then_some(now);
    let n = conn.execute(
        "UPDATE transfers SET state = ?3, finished_at = ?4 \
         WHERE owner_key = ?1 AND attachment_id = ?2",
        params![owner_key, attachment_id, state.as_str(), finished],
    )?;
    Ok(n == 1)
}

/// One transfer.
///
/// # Errors
/// The query failed or a stored row is malformed.
pub fn get(
    conn: &Connection,
    owner_key: &str,
    attachment_id: &str,
) -> rusqlite::Result<Option<TransferRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM transfers WHERE owner_key = ?1 AND attachment_id = ?2"),
        params![owner_key, attachment_id],
        read,
    )
    .optional()
}

/// Every transfer, newest first.
///
/// # Errors
/// The query failed or a stored row is malformed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<TransferRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM transfers WHERE owner_key = ?1 \
         ORDER BY started_at DESC, attachment_id"
    ))?;
    let rows = stmt.query_map(params![owner_key], read)?;
    rows.collect()
}

/// A transfer left `active` or `queued` by a Host that stopped (a crash or
/// a quit mid-fetch) is `paused`: nothing runs it, and Start resumes it
/// from its checkpoint. Run at unlock. The number marked.
///
/// # Errors
/// The write failed.
pub fn pause_interrupted(conn: &Connection, owner_key: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE transfers SET state = 'paused' \
         WHERE owner_key = ?1 AND state IN ('active', 'queued')",
        params![owner_key],
    )
}

/// Forget a transfer. `false` when there was none.
///
/// # Errors
/// The write failed.
pub fn remove(conn: &Connection, owner_key: &str, attachment_id: &str) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM transfers WHERE owner_key = ?1 AND attachment_id = ?2",
        params![owner_key, attachment_id],
    )?;
    Ok(n == 1)
}

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod tests;
