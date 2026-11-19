//! An author's edits and deletes of its channel messages (plan E3.3, rules
//! E1–E3), stored on the author and on every reader so they survive a
//! reload. The rules themselves are `rekindle_channel::changes`; each write
//! here repeats its guard in SQL, so a row never takes a change the rules
//! refuse.

use rusqlite::{params, Connection, OptionalExtension as _};

/// Readers cap a message at this many edits (plan E3.3 E2).
pub const MAX_EDITS: u32 = 10;

/// What a change of a stored message is judged against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeTargetRow {
    /// The message's author, as stored.
    pub sender_key: String,
    /// The author seq of the edit it shows; 0 when unedited.
    pub edit_seq: u64,
    /// How many edits we applied.
    pub edit_count: u32,
    /// The author deleted it.
    pub deleted: bool,
}

/// The stored channel message `message_id` a change names, when we hold it.
///
/// # Errors
/// The query failed.
pub fn change_target(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<Option<ChangeTargetRow>> {
    conn.query_row(
        "SELECT sender_key, edit_seq, edit_count, deleted_at IS NOT NULL FROM messages \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND message_id = ?3 LIMIT 1",
        params![owner_key, channel_id, message_id],
        |row| {
            Ok(ChangeTargetRow {
                sender_key: row.get(0)?,
                edit_seq: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                edit_count: u32::try_from(row.get::<_, i64>(2)?).unwrap_or(MAX_EDITS),
                deleted: row.get(3)?,
            })
        },
    )
    .optional()
}

/// One edit to store.
#[derive(Debug, Clone, Copy)]
pub struct StoredEdit<'a> {
    pub owner_key: &'a str,
    pub channel_id: &'a str,
    pub message_id: &'a str,
    /// The edit's author: only the message's own author edits it (E1).
    pub author: &'a str,
    pub body: &'a str,
    pub edited_at: u64,
    /// The edit's author seq: only a later one replaces the shown edit.
    pub seq: u64,
    /// The D26 gate hid the new body.
    pub blurred: bool,
}

/// Store `edit` on its message unless a rule refuses it: another author
/// (E1), a seq at or below the shown edit's (E1), the cap reached (E2), or
/// a deleted message (E3). Whether the row changed.
///
/// # Errors
/// The write failed.
pub fn store_edit(conn: &Connection, edit: &StoredEdit<'_>) -> rusqlite::Result<bool> {
    let seq = i64::try_from(edit.seq).unwrap_or(i64::MAX);
    let edited_at = i64::try_from(edit.edited_at).unwrap_or(i64::MAX);
    let changed = conn.execute(
        "UPDATE messages SET body = ?5, edited_at = ?6, edit_seq = ?7, \
                edit_count = edit_count + 1, automod_blurred = ?8 \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND message_id = ?3 AND sender_key = ?4 COLLATE NOCASE AND deleted_at IS NULL \
           AND edit_seq < ?7 AND edit_count < ?9",
        params![
            edit.owner_key,
            edit.channel_id,
            edit.message_id,
            edit.author,
            edit.body,
            edited_at,
            seq,
            i32::from(edit.blurred),
            MAX_EDITS,
        ],
    )?;
    Ok(changed > 0)
}

/// Tombstone `author`'s message `message_id` (E3): its body, every
/// revision and its attachment record are purged, and the row stays so the
/// author's record never restores it. Whether the row changed (a second
/// delete changes nothing).
///
/// # Errors
/// The write failed.
pub fn tombstone(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
    author: &str,
    deleted_at: u64,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE messages SET body = '', edited_at = NULL, attachment_json = NULL, deleted_at = ?5 \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND message_id = ?3 AND sender_key = ?4 COLLATE NOCASE AND deleted_at IS NULL",
        params![
            owner_key,
            channel_id,
            message_id,
            author,
            i64::try_from(deleted_at).unwrap_or(i64::MAX)
        ],
    )?;
    Ok(changed > 0)
}

/// One moderator's delete to store (plan E3.3 M4).
#[derive(Debug, Clone, Copy)]
pub struct AdminTombstone<'a> {
    pub owner_key: &'a str,
    pub channel_id: &'a str,
    pub message_id: &'a str,
    /// The moderator's pseudonym, shown as "removed by".
    pub by: &'a str,
    pub deleted_at: u64,
}

/// Tombstone a member's message for a moderator's `AdminDelete`: whoever
/// wrote it, its body, every revision and its attachment record are
/// purged and `by` is kept. Delete wins: a row already deleted (by its
/// author or a moderator) does not change. Whether the row changed.
///
/// # Errors
/// The write failed.
pub fn admin_tombstone(conn: &Connection, delete: &AdminTombstone<'_>) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE messages SET body = '', edited_at = NULL, attachment_json = NULL, \
                deleted_at = ?5, deleted_by = ?4 \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
           AND message_id = ?3 AND deleted_at IS NULL",
        params![
            delete.owner_key,
            delete.channel_id,
            delete.message_id,
            delete.by,
            i64::try_from(delete.deleted_at).unwrap_or(i64::MAX)
        ],
    )?;
    Ok(changed > 0)
}

#[cfg(test)]
#[path = "channel_changes_tests.rs"]
mod tests;
