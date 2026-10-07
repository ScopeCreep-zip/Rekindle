//! Phase 23.C — `load_friends_from_db` lifted from `commands/auth.rs`.

use crate::db_helpers::db_call;
use crate::state::{FriendState, FriendshipState, SharedState, UserStatus};
use rekindle_db::Db;

/// Load friends from `SQLite` into `AppState`, scoped to the given identity.
pub async fn load_friends_from_db(
    pool: &Db,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    let ok = owner_key.to_string();
    let rows = db_call(pool, move |conn| {
        rekindle_db::repo::friends::load_all(conn, &ok)
    })
    .await?;

    let mut friends = state.friends.write();
    for row in rows {
        let friendship_state = match row.friendship_state.as_str() {
            "pending_out" => FriendshipState::PendingOut,
            _ => FriendshipState::Accepted,
        };
        friends.insert(
            row.public_key.clone(),
            FriendState {
                public_key: row.public_key,
                display_name: row.display_name,
                nickname: row.nickname,
                status: UserStatus::Offline,
                status_message: None,
                game_info: None,
                group: row.group_name,
                unread_count: 0,
                dht_record_key: row.dht_record_key,
                last_seen_at: row.last_seen_at,
                local_conversation_key: row.local_conversation_key,
                remote_conversation_key: row.remote_conversation_key,
                mailbox_dht_key: row.mailbox_dht_key,
                last_heartbeat_at: None,
                friendship_state,
            },
        );
    }
    Ok(())
}
