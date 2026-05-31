//! Phase 23.C — friend-group SQLite handlers lifted from
//! `commands/friends.rs`. Tiny CRUD orchestrators around the
//! `friend_groups` table.

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

pub async fn create_friend_group_inner(
    state: &SharedState,
    pool: &DbPool,
    name: String,
) -> Result<i64, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO friend_groups (owner_key, name) VALUES (?1, ?2)",
            rusqlite::params![owner_key, name],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .await
}

pub async fn rename_friend_group_inner(
    pool: &DbPool,
    group_id: i64,
    name: String,
) -> Result<(), String> {
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE friend_groups SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, group_id],
        )?;
        Ok(())
    })
    .await
}
