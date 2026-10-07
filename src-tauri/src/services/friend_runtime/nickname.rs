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

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Set (or clear, with `None`) the local alias for one friend.
pub async fn set_friend_nickname_inner(
    state: &Arc<AppState>,
    pool: &Db,
    app: &tauri::AppHandle,
    public_key: String,
    nickname: Option<String>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;

    let (pk, ok, nick) = (public_key.clone(), owner_key, nickname.clone());
    db_call(pool, move |conn| {
        rekindle_db::repo::friends::set(
            conn,
            &ok,
            &pk,
            rekindle_db::repo::friends::Column::Nickname,
            nick,
        )
    })
    .await?;

    if let Some(f) = state.friends.write().get_mut(&public_key) {
        f.nickname.clone_from(&nickname);
    }

    // The DHT half. The friend list is our own record, held writable by
    // the session's record pool, so this is a plain owner write.
    let key = state
        .node
        .read()
        .as_ref()
        .and_then(|nh| nh.friend_list_dht_key.clone());
    if let (Some(key), Ok(pool)) = (key, crate::state_helpers::record_pool(state)) {
        // `group: None` on purpose — renaming a friend and moving them
        // between groups are separate user actions, and `update_friend`
        // already treats `None` as "leave this field alone".
        match rekindle_protocol::dht::friends::update_friend(
            &pool,
            &key,
            &public_key,
            nickname.clone(),
            None,
        )
        .await
        {
            Ok(outcome) if !outcome.missed() => {}
            Ok(outcome) => {
                tracing::warn!(?outcome, "friend nickname not stored at consensus");
            }
            Err(e) => tracing::warn!(error = %e, "friend nickname DHT write failed"),
        }
    }

    crate::event_dispatch::emit_subscription(
        app,
        &rekindle_types::subscription_events::SubscriptionEvent::Friend(
            rekindle_types::subscription_events::FriendEvent::NicknameChanged {
                peer_key: public_key,
                nickname,
            },
        ),
    );
    Ok(())
}
