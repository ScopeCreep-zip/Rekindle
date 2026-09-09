//! Per-friend local nickname (architecture: friend alias).
//!
//! `FriendEntry.nickname` has existed in the DHT friend-list entry, the
//! `friends` SQLite table, `state::friend`, and the frontend's
//! `friends.store.ts` — set to `None` at every construction site,
//! because nothing could change it. `set_nickname` is the *user's own*
//! display name, a different thing.
//!
//! `rekindle_protocol::dht::friends::update_friend` is the DHT half,
//! written and deliberately kept unwired until this caller existed.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

/// Set (or clear, with `None`) the local alias for one friend.
pub async fn set_friend_nickname_inner(
    state: &Arc<AppState>,
    pool: &DbPool,
    app: &tauri::AppHandle,
    public_key: String,
    nickname: Option<String>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;

    let (pk, ok, nick) = (public_key.clone(), owner_key, nickname.clone());
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE friends SET nickname = ?3 WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk, nick],
        )?;
        Ok(())
    })
    .await?;

    if let Some(f) = state.friends.write().get_mut(&public_key) {
        f.nickname.clone_from(&nickname);
    }

    // The DHT half. The friend list is our own record, so this is a
    // plain owner write. There is no `state_helpers::dht_manager` — the
    // established pattern (dht_publish_service.rs, profile_push.rs) is
    // to take the routing context and build a `DHTManager` around it.
    let target = {
        let node = state.node.read();
        node.as_ref().and_then(|nh| {
            nh.friend_list_dht_key
                .clone()
                .map(|key| (key, nh.routing_context.clone()))
        })
    };
    if let Some((key, rc)) = target {
        let dht = rekindle_protocol::dht::DHTManager::new(rc);
        // `group: None` on purpose — renaming a friend and moving them
        // between groups are separate user actions, and `update_friend`
        // already treats `None` as "leave this field alone".
        if let Err(e) = rekindle_protocol::dht::friends::update_friend(
            &dht,
            &key,
            &public_key,
            nickname.clone(),
            None,
        )
        .await
        {
            tracing::warn!(error = %e, "friend nickname DHT write failed");
        }
    }

    crate::event_dispatch::emit_live(
        app,
        "friend-event",
        &serde_json::json!({
            "type": "nicknameChanged",
            "data": { "publicKey": public_key, "nickname": nickname }
        }),
    );
    Ok(())
}
