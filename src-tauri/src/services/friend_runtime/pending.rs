//! Phase 23.C — pending-friend-request SQLite scan lifted from
//! `commands/friends.rs`. Returns the persisted pending requests for
//! the current owner key, ordered by `received_at`.

pub use rekindle_db::repo::pending_requests::PendingFriendRequest;

use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;

pub async fn get_pending_requests_inner(
    state: &SharedState,
    pool: &Db,
) -> Result<Vec<PendingFriendRequest>, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        rekindle_db::repo::pending_requests::list(conn, &owner_key)
    })
    .await
}
