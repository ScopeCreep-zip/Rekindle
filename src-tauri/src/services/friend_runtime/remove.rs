//! Phase 23.C — split from friend_runtime.rs. remove_friend orchestration.

use std::sync::Arc;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::services;
use crate::state::{AppState, FriendshipState};
use crate::state_helpers;

pub async fn remove_friend_inner(
    state: Arc<AppState>,
    pool: DbPool,
    app: tauri::AppHandle,
    public_key: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;

    let _ = services::message_service::build_and_queue_envelope(
        &state,
        &pool,
        &public_key,
        &rekindle_protocol::messaging::envelope::MessagePayload::Unfriended,
    )
    .await;

    let pk = public_key.clone();
    let ok = owner_key.clone();
    db_call(&pool, move |conn| {
        crate::friend_repo::delete_friend(conn, &ok, &pk)?;
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    crate::audit_repo::append_async(
        &state,
        &pool,
        &owner_key,
        rekindle_audit::AuditKind::FriendRemoved,
        serde_json::json!({ "peer_public_key": public_key }),
    )
    .await;

    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(&public_key) {
            friend.friendship_state = FriendshipState::Removing;
        }
    }

    crate::event_dispatch::emit_live(
        &app,
        "chat-event",
        &ChatEvent::FriendRemoved {
            public_key: public_key.clone(),
        },
    );

    tracing::info!(public_key = %public_key, "friend removed");

    let state_clone = Arc::clone(&state);
    let pool_clone = pool.clone();
    let pk_clone = public_key.clone();
    tokio::spawn(async move {
        if let Err(e) = services::message_service::send_to_peer_raw(
            &state_clone,
            &pool_clone,
            &pk_clone,
            &rekindle_protocol::messaging::envelope::MessagePayload::Unfriended,
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
                mgr.manager.invalidate_route_for_peer(&pk_clone);
            }
        }

        if let Err(e) = services::message_service::push_friend_list_update(&state_clone).await {
            tracing::warn!(error = %e, "failed to update DHT friend list after removal");
        }

        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        let mut friends = state_clone.friends.write();
        if friends
            .get(&pk_clone)
            .is_some_and(|f| matches!(f.friendship_state, FriendshipState::Removing))
        {
            friends.remove(&pk_clone);
            let signal = state_clone.signal_manager.read();
            if let Some(handle) = signal.as_ref() {
                let _ = handle.manager.delete_session(&pk_clone);
            }
            tracing::debug!(public_key = %pk_clone, "cleaned up Removing friend after grace period");
        }
    });

    Ok(())
}
