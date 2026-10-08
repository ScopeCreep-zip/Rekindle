//! Phase 23.C — block_user orchestration lifted from
//! `commands/friends.rs`. Resolve display name, DB transaction
//! (delete friend + pending request + insert blocked row), drop
//! pending messages, unregister DHT key + invalidate route, delete
//! Signal session, rotate profile DHT key, emit FriendRemoved.

use std::sync::Arc;

use crate::db;
use crate::db_helpers::db_call;
use crate::services;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::rotate_profile_key;

pub async fn block_user_inner(
    state: Arc<AppState>,
    pool: Db,
    app: tauri::AppHandle,
    public_key: String,
    display_name: Option<String>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;
    let timestamp = db::timestamp_now();

    let resolved_name = state_helpers::friend_display_name(&state, &public_key)
        .or_else(|| display_name.clone())
        .unwrap_or_else(|| {
            if public_key.len() > 12 {
                format!("{}...", rekindle_utils::text::prefix(&public_key, 12))
            } else {
                public_key.clone()
            }
        });

    let pk = public_key.clone();
    let ok = owner_key;
    let dn = resolved_name;
    db_call(&pool, move |conn| {
        rekindle_db::repo::friends::delete(conn, &ok, &pk)?;
        rekindle_db::repo::pending_requests::delete(conn, &ok, &pk)?;
        conn.execute(
            "INSERT OR REPLACE INTO blocked_users (owner_key, public_key, display_name, blocked_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ok, pk, dn, timestamp],
        )?;
        Ok(())
    })
    .await?;

    services::message_service::delete_pending_messages_to_recipient(&state, &pool, &public_key);

    let dht_key = {
        let mut friends = state.friends.write();
        let removed = friends.remove(&public_key);
        removed.and_then(|f| f.dht_record_key)
    };
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            if let Some(ref dht_key) = dht_key {
                mgr.unregister_friend_dht_key(dht_key);
            }
        }
    }
    state_helpers::invalidate_cached_peer_route(&state, &public_key);

    // Releasing the friend's lease closes the record, which is what
    // cancels its watch. Blocking someone while still watching their
    // presence is the worse half of this defect: the user explicitly cut
    // them off.
    state_helpers::release_friend_record(&state, &public_key).await;

    {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            let _ = handle.manager.delete_session(&public_key);
        }
    }

    rotate_profile_key(&state, &pool).await?;

    crate::event_dispatch::emit_subscription(
        &app,
        &rekindle_types::subscription_events::SubscriptionEvent::Friend(
            rekindle_types::subscription_events::FriendEvent::Removed {
                peer_key: public_key.clone(),
            },
        ),
    );

    tracing::info!(public_key = %public_key, "user blocked and profile key rotated");
    Ok(())
}
