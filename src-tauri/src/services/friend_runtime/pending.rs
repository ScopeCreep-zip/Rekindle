//! Phase 23.C — pending-friend-request SQLite scan lifted from
//! `commands/friends.rs`. Returns the persisted pending requests for
//! the current owner key, ordered by `received_at`.

use serde::{Deserialize, Serialize};

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

/// A pending friend request stored in `SQLite`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFriendRequest {
    pub public_key: String,
    pub display_name: String,
    pub message: String,
    pub received_at: i64,
}

pub async fn get_pending_requests_inner(
    state: &SharedState,
    pool: &DbPool,
) -> Result<Vec<PendingFriendRequest>, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT public_key, display_name, message, received_at \
             FROM pending_friend_requests WHERE owner_key = ?1 ORDER BY received_at",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
            Ok(PendingFriendRequest {
                public_key: row.get(0)?,
                display_name: row.get(1)?,
                message: row.get(2)?,
                received_at: row.get(3)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    })
    .await
}
