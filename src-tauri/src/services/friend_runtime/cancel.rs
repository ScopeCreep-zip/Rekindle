//! Phase 23.C — cancel_request orchestration lifted from
//! `commands/friends.rs`. Verify pending_out, delete DB row, drop
//! from AppState, emit FriendRemoved.

use std::sync::Arc;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::{AppState, FriendshipState};
use crate::state_helpers;

pub async fn cancel_request_inner(
    state: Arc<AppState>,
    pool: DbPool,
    app: tauri::AppHandle,
    public_key: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;

    let is_pending = state
        .friends
        .read()
        .get(&public_key)
        .is_some_and(|f| f.friendship_state == FriendshipState::PendingOut);
    if !is_pending {
        return Err("Not a pending outbound request".to_string());
    }

    let pk = public_key.clone();
    let ok = owner_key;
    db_call(&pool, move |conn| {
        crate::friend_repo::delete_friend(conn, &ok, &pk)
    })
    .await?;

    state.friends.write().remove(&public_key);

    crate::event_dispatch::emit_live(
        &app,
        "chat-event",
        &ChatEvent::FriendRemoved {
            public_key: public_key.clone(),
        },
    );

    tracing::info!(public_key = %public_key, "pending friend request cancelled");
    Ok(())
}
