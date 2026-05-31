use crate::state::AppState;

async fn close_tracked_records(state: &AppState, reason: &str) {
    let rc_and_keys = {
        let node = state.node.read();
        let rc = node.as_ref().map(|nh| nh.routing_context.clone());
        let keys: Vec<String> = {
            let dht_mgr = state.dht_manager.read();
            dht_mgr
                .as_ref()
                .map(|mgr| mgr.open_records.iter().cloned().collect())
                .unwrap_or_default()
        };
        rc.map(|rc| (rc, keys))
    };
    if let Some((rc, keys)) = rc_and_keys {
        tracing::debug!(count = keys.len(), reason, "closing open DHT records");
        for key_str in &keys {
            if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                if let Err(e) = rc.close_dht_record(record_key).await {
                    tracing::trace!(key = %key_str, error = %e, reason, "close DHT record");
                }
            }
        }
    }
}

/// Clean up user-specific state on logout without shutting down the Veilid node.
pub async fn logout_cleanup(app_handle: Option<&tauri::AppHandle>, state: &AppState) {
    crate::services::voice_adapter::shutdown_voice(
        state,
        &rekindle_voice::VoiceShutdownOpts::FULL,
    )
    .await;

    {
        let tx = state.idle_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }
    *state.pre_away_status.write() = None;

    {
        let tx = state.heartbeat_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    close_tracked_records(state, "logout").await;

    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during logout");
            }
        }
    }

    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.dht_key_to_friend.clear();
            mgr.conversation_key_to_friend.clear();
            mgr.open_records.clear();
            mgr.manager.route_cache.clear();
            mgr.manager.imported_routes.clear();
            mgr.manager.route_id_to_pubkey.clear();
            mgr.manager.profile_key = None;
            mgr.manager.friend_list_key = None;
        }
    }

    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.route_blob = None;
            nh.profile_dht_key = None;
            nh.profile_owner_keypair = None;
            nh.friend_list_dht_key = None;
            nh.friend_list_owner_keypair = None;
            nh.account_dht_key = None;
            nh.mailbox_dht_key = None;
        }
    }

    if let Some(ah) = app_handle {
        super::status::emit_network_status(ah, state);
    }

    // Final flush of in-memory peer-reliability counters before we drop
    // the per-community state. Skipped silently if no app handle / pool.
    if let Some(ah) = app_handle {
        use tauri::Manager as _;
        if let Some(pool_state) = ah.try_state::<crate::db::DbPool>() {
            crate::services::community::flush_peer_reliability(state, pool_state.inner()).await;
        }
    }

    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();
    *state.signal_manager.write() = None;
    *state.identity_secret.lock() = None;
    // Phase 4 — drop the audit chain so a subsequent login under a
    // different identity doesn't append against the prior chain's MAC.
    *state.audit_chain.lock() = None;
    // Phase 7 (consolidated Phase 12) — tear down the friendship
    // inbox-scan coordinator. `FriendshipHandle::shutdown` signals the
    // spawned task to return cleanly and clears the watch trigger's
    // sender; the next login installs a fresh coordinator.
    state.friendship_handle.shutdown();
    // Phase 10 — privacy: drop journaled events from the previous user
    // so a re-login (same or different identity) can't replay them via
    // `event_resume`. Resets the cursor counter to 1 too; the
    // frontend's persisted cursor becomes meaningless against the new
    // journal generation, which is correct — cross-session resume is
    // not a feature.
    state.event_journal.clear();
    // Pair the journal clear with a watermark reset so the next session
    // can replay its own events from cursor 0 — leaving the watermark
    // at its previous-session value would gate every new resume call
    // and silently swallow the next user's backlog.
    *state.event_replay_watermark.lock() = 0;
    state.mek_cache.lock().clear();
    state.channel_mek_cache.lock().clear();
    state.dm_mek_cache.lock().clear();
    state.relay_probe_cooldown.lock().clear();
    state.dedup_cache.lock().clear();

    tracing::info!("logout cleanup complete — node still running");
}

/// Shutdown the Veilid node (called only on app exit).
pub async fn shutdown_app(state: &AppState) {
    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    close_tracked_records(state, "app exit").await;

    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during app exit");
            }
        }
        *rm = None;
    }
    *state.dht_manager.write() = None;

    let api = {
        let mut node = state.node.write();
        node.take().map(|nh| nh.api)
    };
    if let Some(api) = api {
        api.shutdown().await;
    }

    tracing::info!("veilid node shut down");
}
