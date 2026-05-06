use rusqlite::OptionalExtension;

use crate::channels::community_channel::ThreadInfoDto;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

pub async fn persist_thread_row(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    thread: &ThreadInfoDto,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let persisted = thread.clone();
    let community_id = community_id.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO community_threads \
             (owner_key, community_id, id, channel_id, name, starter_message_id, creator_pseudonym, created_at, archived, auto_archive_seconds, last_message_at, message_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                owner_key,
                community_id,
                persisted.id,
                persisted.channel_id,
                persisted.name,
                persisted.starter_message_id,
                persisted.creator_pseudonym,
                persisted.created_at,
                i32::from(persisted.archived),
                persisted.auto_archive_seconds,
                persisted.last_message_at,
                persisted.message_count,
            ],
        )?;
        Ok(())
    })
    .await
}

pub async fn load_thread_metadata(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    thread_id: &str,
) -> Result<Option<ThreadInfoDto>, String> {
    let owner_key = owner_key.to_string();
    let community_id = community_id.to_string();
    let thread_id = thread_id.to_string();
    db_call(pool, move |conn| {
        conn.query_row(
            "SELECT id, channel_id, name, starter_message_id, creator_pseudonym, created_at, archived, auto_archive_seconds, last_message_at, message_count \
             FROM community_threads WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
            rusqlite::params![owner_key, community_id, thread_id],
            |row| {
                Ok(ThreadInfoDto {
                    id: row.get(0)?,
                    channel_id: row.get(1)?,
                    name: row.get(2)?,
                    starter_message_id: row.get(3)?,
                    creator_pseudonym: row.get(4)?,
                    forum_tag: None,
                    created_at: row.get(5)?,
                    archived: row.get::<_, i32>(6)? != 0,
                    auto_archive_seconds: row.get(7)?,
                    last_message_at: row.get(8)?,
                    message_count: row.get(9)?,
                })
            },
        )
        .optional()
    })
    .await
}
