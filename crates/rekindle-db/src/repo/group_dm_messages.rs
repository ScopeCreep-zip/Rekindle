//! A group DM's messages (plan E2.6): `messages` rows under the group's
//! id with `conversation_type = 'group_dm'`, in the 1:1 rows' shape.

use rusqlite::{params, Connection};

use super::messages::{own_message, DmRow, OwnDm, DM_COLS, HISTORY_ORDER};
use crate::row::query_list;

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
    let mut rows: Vec<DmRow> = query_list(
        conn,
        &format!(
            "SELECT {DM_COLS} FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'group_dm' AND timestamp < ?3 \
         ORDER BY {HISTORY_ORDER} LIMIT ?4"
        ),
        params![owner_key, group_id, before.unwrap_or(i64::MAX), limit],
    )?;
    rows.reverse();
    Ok(rows)
}

/// Our message to group `group_id` whose client id is `message_id`, as
/// stored with its parent's client id (plan E6.11: the row our send
/// publishes); `None` when we sent none with that id to that group.
///
/// # Errors
/// The query failed.
pub fn own_group_dm(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    message_id: &str,
) -> rusqlite::Result<Option<OwnDm>> {
    Ok(own_message(conn, owner_key, message_id, "group_dm")?
        .filter(|own| own.row.peer_key == group_id))
}
