use std::sync::Arc;

use tokio::sync::mpsc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

/// Start the periodic sync service.
///
/// Runs in the background, periodically syncing local `SQLite` cache with Veilid DHT.
/// - Pull: Read latest from DHT -> update `SQLite`
/// - Push: Send local changes to DHT
/// - Retry: Attempt to deliver queued pending messages
pub async fn start_sync_loop(
    state: Arc<AppState>,
    pool: DbPool,
    app_handle: tauri::AppHandle,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("sync service started");

    // Start at 10s to give gossip overlay setup a chance to complete before
    // the first rejoin attempt. This ensures peers are discovered via presence
    // scanning before we try to broadcast MemberJoinRequest.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut watched_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut first_tick = true;
    let mut tick_count: u32 = 0;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                tick_count += 1;
                let force_all = tick_count.is_multiple_of(10);
                if let Err(e) = sync_friends(&state, &mut watched_keys, first_tick, force_all).await {
                    tracing::warn!(error = %e, "friend sync failed");
                }
                first_tick = false;
                // After the first 3 rapid ticks, switch to the normal 30s cadence
                if tick_count == 3 {
                    interval = tokio::time::interval(std::time::Duration::from_secs(30));
                }
                if let Err(e) = sync_conversations(&state).await {
                    tracing::warn!(error = %e, "conversation sync failed");
                }
                if let Err(e) = crate::services::sync_communities::sync_communities(&state, &pool).await {
                    tracing::warn!(error = %e, "community sync failed");
                }
                if let Err(e) = retry_pending_messages(&state, &pool).await {
                    tracing::warn!(error = %e, "pending message retry failed");
                }
                // Every ~6th tick (~3 minutes) — expire stale pending requests + invites
                if tick_count.is_multiple_of(6) {
                    expire_stale_requests(&state, &pool, &app_handle).await;
                    let owner_key = state_helpers::owner_key_or_default(&state);
                    if !owner_key.is_empty() {
                        crate::invite_helpers::expire_stale_invites(&pool, &owner_key);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("sync service shutting down");
                break;
            }
        }
    }
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
    sync_friends(state, &mut watched_keys, true, true).await
}

/// Friend-sync tick. Delegates to the crate orchestrator after
/// constructing the presence adapter.
async fn sync_friends(
    state: &Arc<AppState>,
    watched_keys: &mut std::collections::HashSet<String>,
    first_tick: bool,
    force_all: bool,
) -> Result<(), String> {
    let Some(adapter) = crate::services::presence_adapter::build_adapter(state) else {
        return Ok(());
    };
    rekindle_presence::sync_friends(Arc::new(adapter), watched_keys, first_tick, force_all).await;
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
async fn expire_stale_requests(
    state: &Arc<AppState>,
    pool: &DbPool,
    app_handle: &tauri::AppHandle,
) {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return;
    }

    let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
    let cutoff = crate::db::timestamp_now() - thirty_days_ms;

    // 1. Delete expired pending_friend_requests (fire-and-forget).
    let ok = owner_key.clone();
    crate::db_helpers::db_fire(pool, "expire stale incoming requests", move |conn| {
        let deleted = conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND received_at < ?2",
            rusqlite::params![ok, cutoff],
        )?;
        if deleted > 0 {
            tracing::info!(deleted, "expired stale incoming friend requests");
        }
        Ok(())
    });

    // 2. Find and remove expired pending_out friends.
    let ok = owner_key;
    let expired_pending: Vec<String> =
        crate::db_helpers::db_call_or_default(pool, move |conn| {
            let mut stmt = conn.prepare(
                "SELECT public_key FROM friends \
                 WHERE owner_key = ?1 AND friendship_state = 'pending_out' AND added_at < ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![ok, cutoff], |row| row.get::<_, String>(0))?
                .filter_map(std::result::Result::ok)
                .collect::<Vec<_>>();
            for pk in &rows {
                crate::friend_repo::delete_friend(conn, &ok, pk)?;
            }
            Ok(rows)
        })
        .await;

    for pk in &expired_pending {
        state.friends.write().remove(pk);
        crate::event_dispatch::emit_live(
            app_handle,
            "chat-event",
            &crate::channels::ChatEvent::FriendRemoved {
                public_key: pk.clone(),
            },
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
/// opens the conversation record read-only, reads the header, and caches the
/// route blob and profile snapshot.
async fn sync_conversations(state: &Arc<AppState>) -> Result<(), String> {
    let Some(routing_context) = state_helpers::safe_routing_context(state) else {
        return Ok(()); // Not connected yet
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
        sync_single_conversation(
            state,
            &routing_context,
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
    routing_context: &veilid_core::RoutingContext,
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

    // Open the remote conversation record read-only
    let record = match rekindle_protocol::dht::conversation::ConversationRecord::open_read(
        routing_context,
        remote_conv_key,
        encryption_key,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::trace!(
                friend = %friend_key, key = %remote_conv_key,
                error = %e, "failed to open remote conversation record"
            );
            return;
        }
    };

    // Track the opened record key for cleanup on logout/exit
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = dht_mgr.as_mut() {
            mgr.track_open_record(remote_conv_key.to_string());
        }
    }

    // Read header and cache route blob + profile
    match record.read_header().await {
        Ok(header) => {
            // Cache route blob
            if !header.route_blob.is_empty() {
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
        Err(e) => {
            tracing::trace!(
                friend = %friend_key, key = %remote_conv_key,
                error = %e, "failed to read remote conversation header"
            );
        }
    }

    // Best-effort close
    let _ = record.close().await;
}
pub(super) async fn request_channel_sync(
    state: &Arc<AppState>,
    pool: &DbPool,
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

    let sync_req = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::SyncRequest {
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
async fn retry_pending_messages(state: &Arc<AppState>, pool: &DbPool) -> Result<(), String> {
    crate::services::sync_adapter::run_pending_retry_tick(state, pool).await;
    Ok(())
}

