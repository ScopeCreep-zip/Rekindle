//! Phase 23.C — move_friend_to_group orchestrator lifted from
//! `commands/friends.rs`. Updates the SQLite friend row's group_id,
//! then refreshes the in-memory `FriendState.group` label (resolving
//! the group name when group_id is Some).

use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

pub async fn move_friend_to_group_inner(
    state: Arc<AppState>,
    pool: DbPool,
    public_key: String,
    group_id: Option<i64>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;
    let pk = public_key.clone();
    let ok = owner_key;
    db_call(&pool, move |conn| {
        crate::friend_repo::update_group_id(conn, &ok, &pk, group_id)
    })
    .await?;

    if let Some(group_id) = group_id {
        let group_name: Option<String> = db_call(&pool, move |conn| {
            let name = conn
                .query_row(
                    "SELECT name FROM friend_groups WHERE id = ?1",
                    rusqlite::params![group_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(name)
        })
        .await?;

        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(&public_key) {
            friend.group = group_name;
        }
    } else {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(&public_key) {
            friend.group = None;
        }
    }

    Ok(())
}
