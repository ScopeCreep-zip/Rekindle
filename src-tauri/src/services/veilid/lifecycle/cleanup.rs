use crate::state::AppState;

/// Clean up user-specific state on logout without shutting down the Veilid node.
pub async fn logout_cleanup(end: crate::services::session::SessionEnd<'_>, state: &AppState) {
    use crate::services::session::SessionEnd;
    let app_handle = match end {
        SessionEnd::Logout(app) => Some(app),
        SessionEnd::Exit => None,
    };
    crate::services::voice_adapter::shutdown_voice(state, &rekindle_voice::VoiceShutdownOpts::FULL)
        .await;

    *state.pre_away_status.write() = None;
    // A deep link received while this user was logged in is theirs to
    // answer, never the next user's.
    *state.pending_deep_link.lock() = None;

    // This session's routes go, and on a logout fresh ones are allocated
    // for the next login: a route is never shared by two identities (plan
    // C7.9b). A route left allocated would persist in Veilid's table store
    // and keep answering peers' pings. At exit no login follows, and
    // `shutdown_app` releases them for good.
    if let (SessionEnd::Logout(_), Some(routes)) = (end, state.own_routes.read().clone()) {
        routes.renew();
    }

    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.dht_key_to_friend.clear();
            mgr.conversation_key_to_friend.clear();
        }
    }

    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.profile_dht_key = None;
            nh.profile_owner_keypair = None;
            nh.profile_lease = None;
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
    // the per-community state. Skipped when no identity database is open.
    if let Ok(pool) = state.db.current() {
        crate::services::community::flush_peer_reliability(state, &pool).await;
    }

    *state.identity.write() = None;
    *state.game_detector.lock() = None;
    // Pending deliveries belong to the session; its held writes went to the
    // outbox (C7.6h) and land with no status shown (C7.14).
    state.channel_pending_deliveries.lock().clear();
    state.friends.write().clear();
    state.communities.write().clear();
    *state.signal_manager.write() = None;
    *state.identity_secret.lock() = None;
    // Phase 4 — drop the audit chain so a subsequent login under a
    // different identity doesn't append against the prior chain's MAC.
    *state.audit_chain.lock() = None;
    // The friendship coordinator stopped with the login scope; drop its
    // trigger senders so the next login installs fresh ones.
    state.friendship_handle.clear();
    // Privacy: drop journaled events from the previous user so no window
    // of the next session can replay them. Every post-login window is
    // destroyed on logout, so no window holds a sequence number from the
    // old journal generation.
    state.event_journal.clear();
    crate::state_helpers::clear_meks(state);
    state.dm_mek_cache.lock().clear();
    state.relay_probe_cooldown.lock().clear();
    state.dedup_cache.lock().clear();
    state.envelope_replay.lock().clear();
    state.voice_sender_keys.clear();

    tracing::info!("logout cleanup complete — node still running");
}

/// Shutdown the Veilid node (called only on app exit).
pub async fn shutdown_app(state: &AppState) {
    // Release our routes for good, so none persists to half-live after exit.
    let routes = state.own_routes.write().take();
    if let Some(routes) = routes {
        routes.shutdown().await;
    }
    *state.routing_manager.write() = None;
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
