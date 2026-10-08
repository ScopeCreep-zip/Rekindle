use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Start the periodic sync service.
///
/// Runs in the background, periodically syncing local `SQLite` cache with Veilid DHT.
/// - Pull: Read latest from DHT -> update `SQLite`
/// - Push: Send local changes to DHT
/// - Retry: Attempt to deliver queued pending messages
pub async fn start_sync_loop(
    state: Arc<AppState>,
    pool: Db,
    app_handle: tauri::AppHandle,
    stop: CancellationToken,
) {
    tracing::info!("sync service started");

    // Start at 10s to give gossip overlay setup a chance to complete before
    // the first rejoin attempt. This ensures peers are discovered via presence
    // scanning before we try to broadcast MemberJoinRequest.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut watched_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut first_tick = true;
    let mut tick_count: u32 = 0;

    // The tick's phases run outside the `select!` and check the token
    // between phases and between items: each is a series of Veilid calls,
    // which are never dropped mid-flight (plan C4.L1). A phase waiting on a
    // record-pool call is released by the pool's drain at logout (C7.6g).
    while stop.run_until_cancelled(interval.tick()).await.is_some() {
        tick_count += 1;
        let force_all = tick_count.is_multiple_of(10);
        if let Err(e) = sync_friends(&state, &mut watched_keys, first_tick, force_all, &stop).await
        {
            tracing::warn!(error = %e, "friend sync failed");
        }
        first_tick = false;
        // After the first 3 rapid ticks, switch to the normal 30s cadence
        if tick_count == 3 {
            interval = tokio::time::interval(std::time::Duration::from_secs(30));
        }
        if stop.is_cancelled() {
            break;
        }
        if let Err(e) = sync_conversations(&state, &stop).await {
            tracing::warn!(error = %e, "conversation sync failed");
        }
        if stop.is_cancelled() {
            break;
        }
        if let Err(e) =
            crate::services::sync_communities::sync_communities(&state, &pool, &stop).await
        {
            tracing::warn!(error = %e, "community sync failed");
        }
        if stop.is_cancelled() {
            break;
        }
        if let Err(e) = retry_pending_messages(&state, &pool, &stop).await {
            tracing::warn!(error = %e, "pending message retry failed");
        }
        // Every ~6th tick (~3 minutes) — expire stale pending requests + invites
        if tick_count.is_multiple_of(6) && !stop.is_cancelled() {
            expire_stale_requests(&state, &pool, &app_handle).await;
            let owner_key = state_helpers::owner_key_or_default(&state);
            if !owner_key.is_empty() {
                crate::invite_helpers::expire_stale_invites(&pool, &owner_key);
            }
        }
    }
    tracing::info!("sync service shutting down");
}

/// Run a single friend sync immediately (called from auth on login).
/// Avoids waiting 30 seconds for the first periodic tick.
///
/// Phase 22.c-REDO — the orchestrator (iterate friends + register
/// + watch + force-poll subkeys + check stale) lives in
/// `rekindle_presence::sync_friends`. This facade builds the
/// adapter and delegates.
pub async fn sync_friends_now(
    state: &Arc<AppState>,
    _app_handle: &tauri::AppHandle,
) -> Result<(), String> {
    let mut watched_keys = std::collections::HashSet::new();
    let stop = state_helpers::login_scope_or_closed(state).token();
    sync_friends(state, &mut watched_keys, true, true, &stop).await
}

/// Friend-sync tick. Delegates to the crate orchestrator after
/// constructing the presence adapter.
async fn sync_friends(
    state: &Arc<AppState>,
    watched_keys: &mut std::collections::HashSet<String>,
    first_tick: bool,
    force_all: bool,
    stop: &CancellationToken,
) -> Result<(), String> {
    let Some(adapter) = crate::services::presence_adapter::build_adapter(state) else {
        return Ok(());
    };
    // Friend sync is record-pool work only (the friend deps make no sends):
    // at logout the pool's drain releases it (plan C7.6g).
    rekindle_presence::sync_friends(Arc::new(adapter), watched_keys, first_tick, force_all, stop)
        .await;
    Ok(())
}

/// Expire stale pending friend requests and pending-out friends.
///
/// - Pending incoming requests older than 30 days are deleted from
///   `pending_friend_requests`.
/// - Pending-out friends older than 30 days are removed from
///   `friends` and the frontend is notified.
///
/// src-tauri-side cleanup: this isn't friend-sync (which lives in
/// `rekindle_presence::sync_friends`) — it's pure SQLite + state
/// pruning the periodic sync loop fires every 6th tick.
async fn expire_stale_requests(state: &Arc<AppState>, pool: &Db, app_handle: &tauri::AppHandle) {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return;
    }

    let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
    let cutoff = crate::db::timestamp_now() - thirty_days_ms;

    // 1. Delete expired pending_friend_requests (fire-and-forget).
    let ok = owner_key.clone();
    crate::db_helpers::db_fire(pool, "expire stale incoming requests", move |conn| {
        let deleted =
            rekindle_db::repo::pending_requests::delete_received_before(conn, &ok, cutoff)?;
        if deleted > 0 {
            tracing::info!(deleted, "expired stale incoming friend requests");
        }
        Ok(())
    });

    // 2. Find and remove expired pending_out friends.
    let ok = owner_key;
    let expired_pending: Vec<String> = crate::db_helpers::db_call_or_default(pool, move |conn| {
        rekindle_db::repo::friends::delete_pending_out_before(conn, &ok, cutoff)
    })
    .await;

    for pk in &expired_pending {
        state.friends.write().remove(pk);
        crate::event_dispatch::emit_subscription(
            app_handle,
            &rekindle_types::subscription_events::SubscriptionEvent::Friend(
                rekindle_types::subscription_events::FriendEvent::Removed {
                    peer_key: pk.clone(),
                },
            ),
        );
    }
    if !expired_pending.is_empty() {
        tracing::info!(
            count = expired_pending.len(),
            "expired stale pending-out friends",
        );
    }
}

/// Sync conversation records for friends that have remote conversation keys.
///
/// For each friend with a `remote_conversation_key`, derives the DH shared secret,
/// reads the conversation record's header through the record pool, and caches the
/// route blob and profile snapshot.
async fn sync_conversations(state: &Arc<AppState>, stop: &CancellationToken) -> Result<(), String> {
    let Ok(record_pool) = state_helpers::record_pool(state) else {
        return Ok(()); // Not logged in
    };

    let Some(secret_bytes) = *state.identity_secret.lock() else {
        return Ok(()); // Not logged in
    };

    // Collect friends that have remote conversation keys
    let friends_with_conversations: Vec<(String, String)> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter_map(|f| {
                f.remote_conversation_key
                    .as_ref()
                    .map(|k| (f.public_key.clone(), k.clone()))
            })
            .collect()
    };

    for (friend_key, remote_conv_key) in &friends_with_conversations {
        if stop.is_cancelled() {
            return Ok(());
        }
        sync_single_conversation(
            state,
            &record_pool,
            &secret_bytes,
            friend_key,
            remote_conv_key,
        )
        .await;
    }

    if !friends_with_conversations.is_empty() {
        tracing::debug!(
            conversations = friends_with_conversations.len(),
            "conversation sync complete"
        );
    }

    Ok(())
}

/// Sync a single friend's conversation record from DHT.
async fn sync_single_conversation(
    state: &Arc<AppState>,
    record_pool: &rekindle_protocol::dht::pool::RecordPool,
    my_secret_bytes: &[u8; 32],
    friend_key: &str,
    remote_conv_key: &str,
) {
    let my_identity = rekindle_crypto::Identity::from_secret_bytes(my_secret_bytes);
    let my_x25519_secret = my_identity.to_x25519_secret();

    // Derive the DH conversation encryption key
    let Ok(friend_ed_bytes) = hex::decode(friend_key) else {
        return;
    };
    let Ok(friend_ed_array): Result<[u8; 32], _> = friend_ed_bytes.try_into() else {
        return;
    };
    let friend_identity = rekindle_crypto::Identity::from_secret_bytes(&friend_ed_array);
    let friend_x25519_public = friend_identity.to_x25519_public();

    let encryption_key = rekindle_crypto::DhtRecordKey::derive_conversation_key(
        &my_x25519_secret,
        &friend_x25519_public,
    );

    // Read header and cache route blob + profile
    match rekindle_protocol::dht::conversation::read_conversation_header(
        record_pool,
        remote_conv_key,
        &encryption_key,
    )
    .await
    {
        Ok(Some(header)) => {
            // The header's route blob is a snapshot from conversation
            // creation — the rotation path re-publishes profile
            // subkey 6 + mailbox, never conversation headers — so it
            // is a last-resort fallback only. It must not override
            // fresher route knowledge from the subkey-6 watch
            // (cache_peer_route also heals an active call's voice
            // roster, which a stale blob would poison every tick).
            if !header.route_blob.is_empty()
                && state_helpers::cached_route_blob(state, friend_key).is_none()
            {
                state_helpers::cache_peer_route(state, friend_key, header.route_blob);
            }

            // Update friend display name from conversation profile snapshot
            {
                let mut friends = state.friends.write();
                if let Some(friend) = friends.get_mut(friend_key) {
                    if !header.profile.display_name.is_empty() {
                        friend.display_name = header.profile.display_name;
                    }
                    if !header.profile.status_message.is_empty() {
                        friend.status_message = Some(header.profile.status_message);
                    }
                }
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::trace!(
                friend = %friend_key, key = %remote_conv_key,
                error = %e, "failed to read remote conversation header"
            );
        }
    }
}

pub(super) async fn request_channel_sync(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    channel_id: &str,
) {
    let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
    let ch = channel_id.to_string();
    let last_ts: i64 = db_call(pool, move |conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(timestamp), 0) FROM messages \
             WHERE owner_key=? AND conversation_id=? AND conversation_type='channel'",
            rusqlite::params![owner_key, ch],
            |r| r.get(0),
        )
    })
    .await
    .unwrap_or(0);

    let sync_req = rekindle_codec::community::envelope::CommunityEnvelope::Control(
        rekindle_codec::community::envelope::ControlPayload::SyncRequest {
            channel_id: channel_id.to_string(),
            since_timestamp: last_ts.cast_unsigned(),
        },
    );
    let _ = crate::services::community::send_to_mesh(state, community_id, &sync_req);

    let now = rekindle_utils::timestamp_secs();
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        cs.pending_syncs.insert(channel_id.to_string(), (now, 1));
    }
}

// Route blob publishing is handled by the presence poll loop.

/// Retry sending queued pending messages.
///
/// Phase 22 REDO — the orchestrator (loop + retry-budget decision)
/// lives in `rekindle_sync::process_pending_retry_queue`. The
/// adapter parses the body, dispatches via the appropriate
/// transport, and reports per-row outcomes. This facade just
/// builds the adapter + delegates.
async fn retry_pending_messages(
    state: &Arc<AppState>,
    pool: &Db,
    stop: &CancellationToken,
) -> Result<(), String> {
    crate::services::sync_adapter::run_pending_retry_tick(state, pool, stop).await;
    Ok(())
}
