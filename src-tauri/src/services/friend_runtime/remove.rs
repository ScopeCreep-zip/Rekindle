//! Phase 23.C — split from friend_runtime.rs. remove_friend orchestration.

use std::sync::Arc;

use crate::db_helpers::db_call;
use crate::services;
use crate::state::{AppState, FriendshipState};
use crate::state_helpers;
use rekindle_db::Db;

pub async fn remove_friend_inner(
    state: Arc<AppState>,
    pool: Db,
    app: tauri::AppHandle,
    public_key: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;

    let _ = services::message_service::build_and_queue_envelope(
        &state,
        &pool,
        &public_key,
        &rekindle_codec::message::envelope::MessagePayload::Unfriended,
    )
    .await;

    let pk = public_key.clone();
    let ok = owner_key.clone();
    // The row stays as `removing` while the `Unfriended` notice is queued
    // (its retries need the friend's routing information, across a
    // restart too); `forget_removed_friends` deletes it once nothing is.
    db_call(&pool, move |conn| {
        rekindle_db::repo::friends::set_state(conn, &ok, &pk, FriendshipState::Removing)?;
        rekindle_db::repo::pending_requests::delete(conn, &ok, &pk)
    })
    .await?;

    crate::services::audit::append(
        &state,
        &pool,
        &owner_key,
        rekindle_audit::AuditKind::FriendRemoved,
        serde_json::json!({ "peer_public_key": public_key }),
    );

    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(&public_key) {
            friend.friendship_state = FriendshipState::Removing;
        }
    }

    crate::event_dispatch::emit_subscription(
        &app,
        &rekindle_types::subscription_events::SubscriptionEvent::Friend(
            rekindle_types::subscription_events::FriendEvent::Removed {
                peer_key: public_key.clone(),
            },
        ),
    );

    tracing::info!(public_key = %public_key, "friend removed");

    let state_clone = Arc::clone(&state);
    let pool_clone = pool.clone();
    let pk_clone = public_key.clone();
    crate::state_helpers::spawn_in_login(&state, "friend removal cleanup", async move {
        if let Err(e) = services::message_service::send_to_peer(
            &state_clone,
            &pool_clone,
            &pk_clone,
            &rekindle_codec::message::envelope::MessagePayload::Unfriended,
        )
        .await
        {
            tracing::warn!(to = %pk_clone, error = %e, "failed to send unfriend notification");
        }

        let dht_key = state_helpers::friend_dht_key(&state_clone, &pk_clone);
        {
            let mut dht_mgr = state_clone.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                if let Some(ref dht_key) = dht_key {
                    mgr.unregister_friend_dht_key(dht_key);
                }
            }
        }
        state_helpers::invalidate_cached_peer_route(&state_clone, &pk_clone);

        // Unregistering the mapping above does not stop the watch —
        // it only drops the `dht_key_to_friend` entry. Releasing the
        // friend's lease closes the record, which cancels it.
        state_helpers::release_friend_record(&state_clone, &pk_clone).await;

        if let Err(e) = services::message_service::push_friend_list_update(&state_clone).await {
            tracing::warn!(error = %e, "failed to update DHT friend list after removal");
        }
    });

    Ok(())
}

/// Forget the friends we removed once nothing is queued to them (their
/// `Unfriended` notice was acknowledged or ran out of retries): their rows,
/// their entries in state and their Signal sessions. The sync loop runs it
/// after each retry pass, so a removal that spans a restart still ends.
pub async fn forget_removed_friends(state: &Arc<AppState>, pool: &Db) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let forgotten = db_call(pool, move |conn| {
        rekindle_db::repo::friends::forget_removed(conn, &owner_key)
    })
    .await?;
    for public_key in forgotten {
        state.friends.write().remove(&public_key);
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            if let Err(e) = handle.manager.delete_session(&public_key) {
                tracing::error!(public_key = %public_key, error = %e,
                    "failed to delete Signal session of removed friend");
            }
        }
        tracing::debug!(public_key = %public_key, "forgot removed friend");
    }
    Ok(())
}
