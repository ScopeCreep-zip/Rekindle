//! Phase 23.C — cancel_request orchestration lifted from
//! `commands/friends.rs`. Verify pending_out, delete DB row, drop
//! from AppState, emit FriendRemoved.

use std::sync::Arc;

use crate::db_helpers::db_call;
use crate::state::{AppState, FriendshipState};
use crate::state_helpers;
use rekindle_db::Db;

pub async fn cancel_request_inner(
    state: Arc<AppState>,
    pool: Db,
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
        rekindle_db::repo::friends::delete(conn, &ok, &pk)
    })
    .await?;

    state.friends.write().remove(&public_key);

    crate::event_dispatch::emit_subscription(
        &app,
        &rekindle_types::subscription_events::SubscriptionEvent::Friend(
            rekindle_types::subscription_events::FriendEvent::Removed {
                peer_key: public_key.clone(),
            },
        ),
    );

    tracing::info!(public_key = %public_key, "pending friend request cancelled");
    Ok(())
}
