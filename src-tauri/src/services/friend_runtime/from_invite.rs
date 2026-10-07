//! Phase 23.C — add_friend_from_invite orchestration lifted from
//! `commands/friends.rs`. Decode + verify + recency-check the invite
//! blob, clean up stale state on re-add, DB transaction (DELETE +
//! INSERT into friends with profile/mailbox keys), construct
//! FriendState (PendingOut), cache route + Signal session via
//! `setup_invite_contact`, send friend request via Veilid, start
//! presence watch, emit FriendAdded.

use std::sync::Arc;

use crate::db;
use crate::db_helpers::db_call;
use crate::services;
use crate::state::{AppState, FriendState, FriendshipState, UserStatus};
use crate::state_helpers;
use rekindle_db::Db;

use super::setup_invite_contact;

/// 7-day invite recency window. B11 hardening — even if an attacker
/// harvests a link, the window caps how long the harvest remains
/// useful (vulnerable-user safety stance: leaked links shouldn't
/// grant indefinite reach).
pub const MAX_INVITE_AGE_SECS: u64 = 7 * 24 * 3600;

pub async fn add_friend_from_invite_inner(
    state: Arc<AppState>,
    pool: Db,
    app: tauri::AppHandle,
    invite_string: String,
) -> Result<(), String> {
    let blob = rekindle_protocol::messaging::decode_invite_url(&invite_string)?;
    rekindle_protocol::messaging::verify_invite_blob(&blob)?;
    let now_ms = rekindle_utils::timestamp_ms();
    rekindle_protocol::messaging::check_invite_recency(&blob, now_ms, MAX_INVITE_AGE_SECS)?;

    let owner_key = state_helpers::current_owner_key(&state)?;

    if blob.public_key == owner_key {
        return Err("You cannot add yourself as a friend".to_string());
    }

    if crate::commands::friends::is_user_blocked(&pool, &owner_key, &blob.public_key).await {
        return Err("Cannot add a blocked user. Unblock them first.".to_string());
    }

    let timestamp = db::timestamp_now();

    {
        let is_stale = state_helpers::is_friend(&state, &blob.public_key);
        if is_stale {
            state_helpers::invalidate_cached_peer_route(&state, &blob.public_key);
            state.friends.write().remove(&blob.public_key);
        }
    }

    let pk = blob.public_key.clone();
    let dn = blob.display_name.clone();
    let ok = owner_key;
    let profile_key = blob.profile_dht_key.clone();
    let mailbox_key = blob.mailbox_dht_key.clone();
    db_call(&pool, move |conn| {
        rekindle_db::repo::friends::replace_with_invited(
            conn,
            &ok,
            &pk,
            &dn,
            timestamp,
            &profile_key,
            &mailbox_key,
        )
    })
    .await?;

    let friend = FriendState {
        public_key: blob.public_key.clone(),
        display_name: blob.display_name.clone(),
        nickname: None,
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group: None,
        unread_count: 0,
        dht_record_key: Some(blob.profile_dht_key.clone()),
        last_seen_at: None,
        local_conversation_key: None,
        remote_conversation_key: None,
        mailbox_dht_key: Some(blob.mailbox_dht_key.clone()),
        last_heartbeat_at: None,
        friendship_state: FriendshipState::PendingOut,
    };
    state
        .friends
        .write()
        .insert(blob.public_key.clone(), friend);

    setup_invite_contact(&state, &blob).await;

    services::message_service::send_friend_request(
        &state,
        &pool,
        &blob.public_key,
        "Added via invite link",
        blob.invite_id.as_deref(),
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to send friend request via Veilid");
    });

    if let Err(e) =
        services::presence_service::watch_friend(&state, &blob.public_key, &blob.profile_dht_key)
            .await
    {
        tracing::trace!(error = %e, "failed to watch friend DHT after invite add");
    }

    crate::event_dispatch::emit_subscription(
        &app,
        &rekindle_types::subscription_events::SubscriptionEvent::Friend(
            rekindle_types::subscription_events::FriendEvent::Added {
                peer_key: blob.public_key.clone(),
                display_name: blob.display_name.clone(),
                friendship_state: "pendingOut".to_string(),
            },
        ),
    );

    tracing::info!(public_key = %blob.public_key, "friend added from invite");
    Ok(())
}
