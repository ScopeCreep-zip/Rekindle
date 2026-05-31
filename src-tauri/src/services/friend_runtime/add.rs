//! Phase 23.C — split from friend_runtime.rs. add_friend pre-accept flow.

use std::sync::Arc;

use crate::channels::ChatEvent;
use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::services;
use crate::state::{AppState, FriendState, FriendshipState, UserStatus};
use crate::state_helpers;

/// Add a friend (pending outbound) — SQLite write + AppState insert +
/// Veilid send_friend_request + UI emit + audit-chain append.
///
/// Validates the public key shape, refuses self-add and blocked-user
/// add. The outbound Veilid send is best-effort: failures log but
/// don't block the user-visible flow (the peer may be offline).
pub async fn add_friend_inner(
    state: Arc<AppState>,
    pool: DbPool,
    app: tauri::AppHandle,
    public_key: String,
    display_name: String,
    message: String,
) -> Result<(), String> {
    // Validate public key format (64 hex chars = 32 bytes)
    if public_key.len() != 64 || hex::decode(&public_key).is_err() {
        return Err("Invalid public key — must be a 64-character hex string".to_string());
    }

    let owner_key = state_helpers::current_owner_key(&state)?;

    // Prevent adding yourself
    if public_key == owner_key {
        return Err("You cannot add yourself as a friend".to_string());
    }

    // Prevent adding a blocked user
    if crate::commands::friends::is_user_blocked(&pool, &owner_key, &public_key).await {
        return Err("Cannot add a blocked user. Unblock them first.".to_string());
    }

    let timestamp = db::timestamp_now();

    // Insert into SQLite
    let pk = public_key.clone();
    let dn = display_name.clone();
    let ok = owner_key.clone();
    db_call(&pool, move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at, friendship_state) VALUES (?1, ?2, ?3, ?4, 'pending_out')",
            rusqlite::params![ok, pk, dn, timestamp],
        )?;
        Ok(())
    })
    .await?;

    // Add to in-memory state as pending (not yet accepted by peer)
    let friend = FriendState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        nickname: None,
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group: None,
        unread_count: 0,
        dht_record_key: None,
        last_seen_at: None,
        local_conversation_key: None,
        remote_conversation_key: None,
        mailbox_dht_key: None,
        last_heartbeat_at: None,
        friendship_state: FriendshipState::PendingOut,
    };
    state.friends.write().insert(public_key.clone(), friend);

    // Send friend request via Veilid
    services::message_service::send_friend_request(&state, &pool, &public_key, &message, None)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to send friend request via Veilid (peer may be offline)");
        });

    // Emit event so frontend updates
    crate::event_dispatch::emit_live(
        &app,
        "chat-event",
        &ChatEvent::FriendAdded {
            public_key: public_key.clone(),
            display_name: display_name.clone(),
            friendship_state: "pendingOut".to_string(),
        },
    );

    // Phase 4 — audit chain entry. Best-effort: failures log but don't
    // block the user-visible operation.
    crate::audit_repo::append_async(
        &state,
        &pool,
        &owner_key,
        rekindle_audit::AuditKind::FriendAdded,
        serde_json::json!({
            "peer_public_key": public_key,
            "display_name": display_name,
            "direction": "outbound",
        }),
    )
    .await;

    Ok(())
}
