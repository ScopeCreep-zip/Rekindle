//! Lifecycle dispatch handlers: Status, Unlock, Lock, Shutdown.
//!
//! These are always-available or state-gated commands that manage the
//! daemon's own lifecycle rather than performing Veilid operations.

use std::sync::Arc;

use rekindle_types::display::Check;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use super::DaemonContext;

/// Handle Status — always available, any state.
///
/// Returns the complete `StatusSnapshot` — compact status, subscription system
/// health, and full diagnostic checks. Renderers (CLI/TUI) decide display depth.
pub(crate) fn handle_status(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    use rekindle_types::display::{StatusSnapshot, CircuitSummary};

    let session_guard = ctx.session.read();
    let session = session_guard.as_ref();

    let transport_guard = ctx.transport.read();
    let transport = transport_guard.as_ref();
    let snap = transport.map(|t| t.status_snapshot());

    // Subscription system stats
    let sub_guard = ctx.subscriptions.read();
    let (active_watches, gossip_meshes, gossip_mesh_peers,
         unread_channels, unread_dms, unread_friend_requests,
         poll_loop_active, renewal_loop_active) =
        if let Some(ref sub_mgr) = *sub_guard {
            let meshes = sub_mgr.meshes().read();
            let mesh_peers: usize = meshes.values().map(|m| m.peers.len()).sum();
            (
                sub_mgr.watch_count(),
                meshes.len(),
                mesh_peers,
                sub_mgr.unread_channels().len(),
                sub_mgr.unread_dms().len(),
                sub_mgr.unread_friend_requests(),
                true, true,
            )
        } else {
            (0, 0, 0, 0, 0, 0, false, false)
        };
    drop(sub_guard);

    // Network detail
    let circuit_summary = transport.map_or(
        CircuitSummary { total: 0, healthy: 0, degraded: 0, circuit_open: 0 },
        |t| {
            let peers = t.peers();
            let reg = peers.read();
            reg.circuit_summary()
        },
    );

    // Diagnostic checks
    let checks = build_checks(ctx, state, transport, session);

    let snapshot = StatusSnapshot {
        state: state.as_str().to_string(),
        has_identity: session.is_some(),
        identity_public_key: session.map(|s| s.identity.public_key_hex.clone()),
        identity_display_name: session.map(|s| s.identity.display_name.clone()),
        attachment: snap.as_ref().map_or("unknown".into(), |s| s.attachment.clone()),
        is_attached: snap.as_ref().is_some_and(|s| s.is_attached),
        public_internet_ready: snap.as_ref().is_some_and(|s| s.public_internet_ready),
        uptime_secs: snap.as_ref().map_or(0, |s| s.uptime_secs),
        peer_count: snap.as_ref().map_or(0, |s| s.peer_count),
        route_allocated: snap.as_ref().is_some_and(|s| s.route_allocated),
        route_age_secs: snap.as_ref().and_then(|s| s.route_age_secs),
        active_watches,
        gossip_meshes,
        gossip_mesh_peers,
        unread_channels,
        unread_dms,
        unread_friend_requests,
        dedup_entries: 0,    // TODO: expose from SubscriptionManager
        dedup_suppressed: 0, // TODO: expose from SubscriptionManager
        poll_loop_active,
        renewal_loop_active,
        community_count: session.map_or(0, |s| s.communities.len()),
        friend_count: session.map_or(0, |s| s.pending_friend_requests.len()),
        circuit_summary,
        checks,
    };

    IpcResponse::ok(&snapshot)
}

/// Build all diagnostic checks across all categories.
///
/// Called by `handle_status` to populate `StatusSnapshot.checks`.
/// No category filtering — the daemon always produces the full set.
/// Filtering is a client-side rendering concern.
fn build_checks(
    ctx: &DaemonContext,
    state: DaemonState,
    transport: Option<&Arc<rekindle_transport::TransportNode>>,
    session: Option<&rekindle_transport::Session>,
) -> Vec<Check> {
    let mut checks = Vec::new();

    // ── NODE — daemon process health ────────────────────────────────
    checks.push(if state.can_query() {
        Check::pass("node.state", "node", state.as_str())
    } else if state == DaemonState::Locked {
        Check::warn("node.state", "node", state.as_str())
            .with_description("unlock the daemon: rekindle init")
    } else {
        Check::fail("node.state", "node", state.as_str())
    });

    checks.push(if transport.is_some() {
        Check::pass("node.transport", "node", "started")
    } else {
        Check::fail("node.transport", "node", "not started")
            .with_description("transport failed to start — check Veilid configuration")
    });

    if let Some(t) = transport {
        let snap = t.status_snapshot();
        checks.push(Check::pass("node.uptime", "node", fmt_uptime(snap.uptime_secs)));
    }

    checks.push(if ctx.audit.lock().is_some() {
        Check::pass("node.audit_logger", "node", "active")
    } else {
        Check::warn("node.audit_logger", "node", "inactive")
            .with_description("audit logging disabled — destructive operations not recorded")
    });

    // ── TRANSPORT — Veilid network health ───────────────────────────
    if let Some(t) = transport {
        let snap = t.status_snapshot();

        checks.push(match snap.attachment.as_str() {
            "FullyAttached" | "AttachedStrong" => Check::pass("transport.attachment", "transport", &snap.attachment),
            "AttachedGood" | "AttachedWeak" => Check::warn("transport.attachment", "transport", &snap.attachment),
            _ => Check::fail("transport.attachment", "transport", &snap.attachment)
                .with_description("not attached to Veilid network — check internet connectivity"),
        });

        checks.push(if snap.public_internet_ready {
            Check::pass("transport.public_internet", "transport", "true")
        } else {
            Check::warn("transport.public_internet", "transport", "false")
                .with_description("NAT traversal may be incomplete")
        });

        checks.push(if snap.route_allocated {
            Check::pass("transport.route", "transport", format!("allocated (age: {}s)", snap.route_age_secs.unwrap_or(0)))
        } else {
            Check::warn("transport.route", "transport", "not allocated")
                .with_description("no private route — peers cannot reach this node")
        });

        checks.push(if snap.peer_count > 0 {
            Check::pass("transport.peer_count", "transport", snap.peer_count.to_string())
        } else {
            Check::warn("transport.peer_count", "transport", "0")
                .with_description("no known peers — node may be isolated")
        });
    } else {
        checks.push(Check::fail("transport.status", "transport", "not started")
            .with_description("transport node failed to start"));
    }

    // ── CRYPTO — cryptographic material health ──────────────────────
    checks.push(if ctx.signing_key.read().is_some() {
        Check::pass("crypto.signing_key", "crypto", "loaded")
    } else {
        Check::warn("crypto.signing_key", "crypto", "not loaded")
            .with_description("signing key not in memory — daemon is locked")
    });

    let mek_entries = ctx.mek_cache.read().total_entries();
    let mek_channels = ctx.mek_cache.read().channel_count();
    checks.push(Check::pass("crypto.mek_cache", "crypto",
        format!("{mek_entries} entries across {mek_channels} channels")));

    // ── STORAGE — persistence health ────────────────────────────────
    checks.push(if ctx.session_path.exists() {
        Check::pass("storage.session_file", "storage", "exists")
    } else {
        Check::warn("storage.session_file", "storage", "missing")
            .with_description("no session file — identity not initialized")
    });

    checks.push(if ctx.session.read().is_some() {
        Check::pass("storage.session_loaded", "storage", "yes")
    } else {
        Check::warn("storage.session_loaded", "storage", "no")
    });

    let config_path = ctx.config_dir.display().to_string();
    checks.push(if ctx.config_dir.exists() {
        Check::pass("storage.config_dir", "storage", &config_path)
    } else {
        Check::warn("storage.config_dir", "storage", &config_path)
            .with_description("config directory missing")
    });

    // ── NETWORK — peer and circuit health ───────────────────────────
    if let Some(t) = transport {
        let peer_reg = t.peers();
        let peers = peer_reg.read();
        let summary = peers.circuit_summary();

        checks.push(if summary.total > 0 {
            Check::pass("network.peers_total", "network", summary.total.to_string())
        } else {
            Check::warn("network.peers_total", "network", "0")
        });
        checks.push(Check::pass("network.peers_healthy", "network", summary.healthy.to_string()));

        if summary.degraded > 0 {
            checks.push(Check::warn("network.peers_degraded", "network", summary.degraded.to_string())
                .with_description("some peers have degraded connections"));
        }
        if summary.circuit_open > 0 {
            checks.push(Check::fail("network.peers_circuit_open", "network", summary.circuit_open.to_string())
                .with_description("circuit breakers tripped — peers unreachable"));
        }
    } else {
        checks.push(Check::fail("network.status", "network", "transport not started"));
    }

    // ── IDENTITY — identity health (no secrets exposed) ─────────────
    if let Some(session) = session {
        checks.push(Check::pass("identity.initialized", "identity", "yes"));

        let pk = &session.identity.public_key_hex;
        let pk_short = if pk.len() > 16 {
            format!("{}...{}", &pk[..8], &pk[pk.len() - 4..])
        } else {
            pk.clone()
        };
        checks.push(Check::pass("identity.public_key", "identity", pk_short));
        checks.push(Check::pass("identity.display_name", "identity", &session.identity.display_name));
        checks.push(Check::pass("identity.communities", "identity", session.communities.len().to_string()));

        let has_profile = !session.identity.profile_dht_key.is_empty();
        let has_mailbox = !session.identity.mailbox_dht_key.is_empty();
        let has_friends = !session.identity.friend_list_dht_key.is_empty();
        let dht_value = format!(
            "profile:{} mailbox:{} friends:{}",
            if has_profile { "ok" } else { "missing" },
            if has_mailbox { "ok" } else { "missing" },
            if has_friends { "ok" } else { "missing" },
        );
        checks.push(if has_profile && has_mailbox && has_friends {
            Check::pass("identity.dht_records", "identity", dht_value)
        } else {
            Check::warn("identity.dht_records", "identity", dht_value)
        });
    } else {
        checks.push(Check::fail("identity.initialized", "identity", "no")
            .with_description("run: rekindle init"));
    }

    // ── SUBSCRIPTIONS — event system health ─────────────────────────
    let sub_guard = ctx.subscriptions.read();
    if let Some(ref sub_mgr) = *sub_guard {
        checks.push(Check::pass("subscriptions.watches", "subscriptions", sub_mgr.watch_count().to_string()));
        let meshes = sub_mgr.meshes().read();
        let mesh_peers: usize = meshes.values().map(|m| m.peers.len()).sum();
        checks.push(Check::pass("subscriptions.gossip_meshes", "subscriptions",
            format!("{} meshes, {} peers", meshes.len(), mesh_peers)));
        checks.push(Check::pass("subscriptions.poll_loop", "subscriptions", "active"));
        checks.push(Check::pass("subscriptions.renewal_loop", "subscriptions", "active"));
    } else {
        checks.push(Check::warn("subscriptions.status", "subscriptions", "not initialized")
            .with_description("subscription manager created during unlock"));
    }

    checks
}

/// Format seconds as human-readable uptime.
fn fmt_uptime(secs: u64) -> String {
    if secs < 60 { return format!("{secs}s"); }
    let mins = secs / 60;
    if mins < 60 { return format!("{mins}m {}s", secs % 60); }
    let hours = mins / 60;
    if hours < 24 { return format!("{hours}h {}m", mins % 60); }
    let days = hours / 24;
    format!("{days}d {}h", hours % 24)
}

/// Handle Unlock — transition from Locked → Resuming → Operational.
pub(crate) async fn handle_unlock(
    ctx: &DaemonContext,
    state: DaemonState,
    _passphrase: &str,
) -> IpcResponse {
    if !state.can_unlock() {
        return IpcResponse::error(
            409,
            format!("cannot unlock in state '{}'", state.as_str()),
        );
    }

    ctx.lifecycle.transition(DaemonState::Resuming);

    // Load signing key from OS keyring into memory.
    let signing_key = match crate::state::keystore::load_signing_key().await {
        Ok(handle) => handle,
        Err(e) => {
            ctx.lifecycle.transition(DaemonState::Locked);
            return IpcResponse::error_with_remediation(
                500,
                format!("failed to load signing key: {e}"),
                "ensure identity is initialized: rekindle init",
            );
        }
    };
    let signing_bytes = *signing_key.as_bytes();
    *ctx.signing_key.write() = Some(signing_key);

    // Load friend list keypair from keyring and inject into session before resume.
    // The keypair is stored during identity creation with label "friend_list".
    // Resume needs it to open the friend list record writable.
    if let Ok(Some(fl_kp_bytes)) = crate::state::keystore::load_keypair_bytes("friend_list").await {
        let mut guard = ctx.session.write();
        if let Some(ref mut s) = *guard {
            s.identity.friend_list_keypair_bytes = Some(fl_kp_bytes);
        }
    }

    // Clone transport and session references before the await point.
    // parking_lot::RwLockReadGuard must not be held across await.
    let transport_clone = ctx.transport.read().clone();
    let session_clone = ctx.session.read().clone();
    if let (Some(transport), Some(session)) = (&transport_clone, &session_clone) {
        if let Err(e) = transport.resume(session, &signing_bytes).await {
            tracing::warn!(error = %e, "session resume failed — entering degraded state");
            ctx.lifecycle.transition(DaemonState::Degraded);
            return IpcResponse::ok(&serde_json::json!({
                "state": "degraded",
                "warning": format!("resume failed: {e}"),
            }));
        }
    }

    // Initialize subscription manager (three-tier inbound: watch + gossip + poll)
    if let (Some(transport), Some(session)) = (&transport_clone, &session_clone) {
        let mut sub_mgr = rekindle_transport::SubscriptionManager::new(
            Arc::clone(transport),
            Arc::clone(&ctx.session),
            Arc::clone(&ctx.mek_cache),
        );
        sub_mgr.setup_identity(session).await;
        for membership in session.communities.values() {
            sub_mgr.setup_community(membership).await;
        }
        for (peer_key, dm_log_key) in &session.dm_log_keys {
            sub_mgr.setup_dm_peer(peer_key, dm_log_key).await;
        }
        sub_mgr.start_renewal_loop();
        sub_mgr.start_poll_loop(60);
        tracing::info!(
            watches = sub_mgr.watch_count(),
            communities = session.communities.len(),
            dm_peers = session.dm_log_keys.len(),
            "subscription manager initialized"
        );
        // Notify the IPC server that events are available for delivery.
        // The server's internal delivery task subscribes via the broadcast sender
        // and routes events through the EventRouter to subscribed connections.
        let event_sender = sub_mgr.event_sender().clone();
        *ctx.subscriptions.write() = Some(sub_mgr);
        let _ = ctx.event_watch_tx.send(Some(event_sender));

        // Initialize broadcast manager (outbound gossip mesh)
        let bcast_mgr = rekindle_transport::BroadcastManager::new(
            Arc::clone(transport),
            Arc::clone(&ctx.session),
            Arc::clone(&ctx.mek_cache),
        );
        for membership in session.communities.values() {
            bcast_mgr.register_mesh(&membership.governance_key);
        }
        tracing::info!(
            communities = session.communities.len(),
            "broadcast manager initialized"
        );
        *ctx.broadcast_mgr.write() = Some(bcast_mgr);
    }

    ctx.lifecycle.transition(DaemonState::Operational);
    IpcResponse::ok(&serde_json::json!({ "state": "operational" }))
}

/// Handle Shutdown — initiate graceful daemon shutdown.
///
/// Responds with Ok *before* the process exits so the client gets confirmation.
/// The actual shutdown is triggered by transitioning to ShuttingDown, which
/// notifies the main event loop via `DaemonLifecycle::shutdown_requested()`.
pub(crate) fn handle_shutdown(ctx: &DaemonContext) -> IpcResponse {
    let state = ctx.lifecycle.state();
    if state == DaemonState::ShuttingDown {
        return IpcResponse::ok(&serde_json::json!({ "state": "already_shutting_down" }));
    }

    tracing::info!("shutdown requested via IPC");

    // Zeroize signing key immediately.
    *ctx.signing_key.write() = None;

    // Transition to ShuttingDown — this notifies the main event loop.
    ctx.lifecycle.transition(DaemonState::ShuttingDown);

    IpcResponse::ok(&serde_json::json!({
        "state": "shutting_down",
        "message": "daemon will exit after draining connections",
    }))
}

/// Handle Lock — transition to Locked, zeroize signing key.
pub(crate) fn handle_lock(ctx: &DaemonContext) -> IpcResponse {
    ctx.lifecycle.transition(DaemonState::Locking);
    // Drop the signing key — ZeroizeOnDrop zeroizes the bytes.
    *ctx.signing_key.write() = None;
    ctx.lifecycle.transition(DaemonState::Locked);
    IpcResponse::ok(&serde_json::json!({ "state": "locked" }))
}
