//! A community's thread rows (plan E3.1 E3): what we know of a thread
//! beyond its governance entry — the message it was opened from and when
//! we stored it. The governance entry stays the authority for its name,
//! tag and archive state.

use rekindle_types::thread::ThreadInfo;
use rusqlite::{params, Connection, OptionalExtension};

/// Store `thread`, replacing the row of the same id.
///
/// # Errors
/// The insert failed.
pub fn upsert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    thread: &ThreadInfo,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO community_threads \
         (owner_key, community_id, id, channel_id, name, starter_message_id, creator_pseudonym, \
          created_at, archived, auto_archive_seconds, last_message_at, message_count) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            owner_key,
            community_id,
            thread.id,
            thread.channel_id,
            thread.name,
            thread.starter_message_id,
            thread.creator_pseudonym,
            thread.created_at.cast_signed(),
            i32::from(thread.archived),
            thread.auto_archive_seconds,
            thread.last_message_at.cast_signed(),
            thread.message_count,
        ],
    )?;
    Ok(())
}

/// The stored row of thread `thread_id`.
///
/// # Errors
/// The query failed.
pub fn get(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    thread_id: &str,
) -> rusqlite::Result<Option<ThreadInfo>> {
    conn.query_row(
        "SELECT id, channel_id, name, starter_message_id, creator_pseudonym, created_at, \
         archived, auto_archive_seconds, last_message_at, message_count \
         FROM community_threads WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
        params![owner_key, community_id, thread_id],
        |row| {
            Ok(ThreadInfo {
                id: row.get(0)?,
                channel_id: row.get(1)?,
                name: row.get(2)?,
                starter_message_id: row.get(3)?,
                creator_pseudonym: row.get(4)?,
                forum_tag: None,
                created_at: row.get::<_, i64>(5)?.cast_unsigned(),
                archived: row.get::<_, i32>(6)? != 0,
                auto_archive_seconds: row.get(7)?,
                last_message_at: row.get::<_, i64>(8)?.cast_unsigned(),
                message_count: row.get(9)?,
            })
        },
    )
    .optional()
}
