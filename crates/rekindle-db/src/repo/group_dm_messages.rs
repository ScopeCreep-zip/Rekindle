//! A group DM's messages (plan E2.6): `messages` rows under the group's
//! id with `conversation_type = 'group_dm'`, in the 1:1 rows' shape.

use rusqlite::{params, Connection};

use super::messages::{dm_row, DmRow, DM_COLS, HISTORY_ORDER};

/// The newest `limit` messages of group `group_id` before `before` (a
/// timestamp, ms; all when `None`), oldest first (plan E2.6).
///
/// # Errors
/// The query failed.
pub fn group_dm_history(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    limit: u32,
    before: Option<i64>,
) -> rusqlite::Result<Vec<DmRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {DM_COLS} FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'group_dm' AND timestamp < ?3 \
         ORDER BY {HISTORY_ORDER} LIMIT ?4"
    ))?;
    let mut rows = stmt
        .query_map(
            params![owner_key, group_id, before.unwrap_or(i64::MAX), limit],
            dm_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.reverse();
    Ok(rows)
}
