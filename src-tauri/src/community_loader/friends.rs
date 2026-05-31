//! Phase 23.C — `load_friends_from_db` lifted from `commands/auth.rs`.

use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::state::{FriendState, SharedState, UserStatus};

/// Load friends from `SQLite` into `AppState`, scoped to the given identity.
pub async fn load_friends_from_db(
    pool: &DbPool,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    let ok = owner_key.to_string();
    let friend_rows = db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT f.public_key, f.display_name, f.nickname, f.dht_record_key, \
                 f.last_seen_at, f.local_conversation_key, f.remote_conversation_key, \
                 f.mailbox_dht_key, f.friendship_state, g.name AS group_name \
                 FROM friends f LEFT JOIN friend_groups g ON f.group_id = g.id \
                 WHERE f.owner_key = ?1",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![ok], |row| {
                let fs_str: String = row
                    .get::<_, String>("friendship_state")
                    .unwrap_or_else(|_| "accepted".to_string());
                let friendship_state = match fs_str.as_str() {
                    "pending_out" => crate::state::FriendshipState::PendingOut,
                    _ => crate::state::FriendshipState::Accepted,
                };
                Ok(FriendState {
                    public_key: db::get_str(row, "public_key"),
                    display_name: db::get_str(row, "display_name"),
                    nickname: db::get_str_opt(row, "nickname"),
                    status: UserStatus::Offline,
                    status_message: None,
                    game_info: None,
                    group: db::get_str_opt(row, "group_name"),
                    unread_count: 0,
                    dht_record_key: db::get_str_opt(row, "dht_record_key"),
                    last_seen_at: row.get::<_, Option<i64>>("last_seen_at").unwrap_or(None),
                    local_conversation_key: db::get_str_opt(row, "local_conversation_key"),
                    remote_conversation_key: db::get_str_opt(row, "remote_conversation_key"),
                    mailbox_dht_key: db::get_str_opt(row, "mailbox_dht_key"),
                    last_heartbeat_at: None,
                    friendship_state,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await?;

    let mut friends = state.friends.write();
    for friend in friend_rows {
        let public_key = friend.public_key.clone();
        friends.insert(public_key, friend);
    }
    Ok(())
}
