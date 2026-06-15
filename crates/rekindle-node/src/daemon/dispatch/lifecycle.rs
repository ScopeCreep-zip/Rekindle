//! Lifecycle dispatch handlers: Status, Unlock, Lock, Shutdown.
//!
//! Four self-contained handlers. Each manages one lifecycle transition.
//! All business logic lives in `ChatService` — these handlers orchestrate
//! vault open/close, transport start/stop, and ChatService construction.
//!
//! Lock discipline: parking_lot::RwLock guards are NEVER held across .await.
//! Pattern: clone Arc from RwLock, drop guard, then await on the cloned Arc.

use std::sync::Arc;
use std::time::Duration;

use rekindle_chat::ChatService;
use rekindle_storage::unlock::passphrase::PassphraseUnlock;
use rekindle_storage::unlock::VaultUnlock;
use rekindle_storage::VaultStore;
use rekindle_transport::veilid::broadcast::node::TransportNode;
use rekindle_transport::veilid::VeilidTransport;
use rekindle_types::daemon::DaemonResponse;
use rekindle_types::display::{Check, CircuitSummary, StatusSnapshot};
use rekindle_types::session_types::SessionMeta;
use rekindle_types::transport::Transport;

use crate::daemon::DaemonState;

use super::DaemonContext;

// ── Transport cleanup guard ────────────────────────────────────────────

struct TransportGuard {
    node: Option<Arc<TransportNode>>,
}

impl TransportGuard {
    fn new(node: Arc<TransportNode>) -> Self {
        Self { node: Some(node) }
    }

    fn disarm(&mut self) {
        self.node = None;
    }
}

impl Drop for TransportGuard {
    fn drop(&mut self) {
        if let Some(node) = self.node.take() {
            tracing::warn!("transport guard triggered — shutting down orphaned transport");
            tokio::spawn(async move {
                node.graceful_shutdown().await;
            });
        }
    }
}

// ── Unlock ─────────────────────────────────────────────────────────────

pub(crate) async fn handle_unlock(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    passphrase: &str,
) -> DaemonResponse {
    // Step 1: Guard
    if !state.can_unlock() {
        return DaemonResponse::error(
            409,
            format!("cannot unlock in state '{}' — daemon must be locked", state.as_str()),
        );
    }

    // Step 2: Transition
    if !ctx.lifecycle.transition(DaemonState::Resuming) {
        return DaemonResponse::error(409, "state transition to Resuming rejected — concurrent state change");
    }

    // Step 3: Derive master key from passphrase
    let passphrase_unlock = PassphraseUnlock::new(&ctx.paths.state_dir, passphrase.as_bytes());
    let is_fresh = !ctx.paths.vault_db.exists();

    let master_key = if is_fresh {
        let mk = match rekindle_storage::unlock::MasterKey::generate() {
            Ok(mk) => mk,
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return DaemonResponse::error(500, format!("master key generation failed: {e}"));
            }
        };
        if let Err(e) = passphrase_unlock.enroll(&mk) {
            ctx.lifecycle.transition(DaemonState::Locked);
            return DaemonResponse::error(500, format!("passphrase enrollment failed: {e}"));
        }
        tracing::info!("fresh node — vault credentials enrolled");
        mk
    } else {
        match passphrase_unlock.unlock() {
            Ok(mk) => mk,
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return DaemonResponse::error_with_remediation(
                    401,
                    format!("unlock failed: {e}"),
                    "check passphrase, or if identity was never initialized: rekindle init",
                );
            }
        }
    };

    // Step 4: Open vault
    let vault = if is_fresh {
        match VaultStore::create(&ctx.paths.vault_db, master_key.as_bytes()) {
            Ok(v) => Arc::new(v),
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return DaemonResponse::error(500, format!("vault creation failed: {e}"));
            }
        }
    } else {
        match VaultStore::open(&ctx.paths.vault_db, master_key.as_bytes()) {
            Ok(v) => Arc::new(v),
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return DaemonResponse::error_with_remediation(
                    500,
                    format!("vault open failed: {e}"),
                    "vault may be corrupt — try: rekindle vault repair",
                );
            }
        }
    };

    // Step 5: Derive session MAC key
    let session_mac_key = rekindle_storage::session_meta::derive_mac_key(master_key.as_bytes());

    // Step 6: Load session.json
    let session_meta: SessionMeta = if is_fresh {
        let meta = SessionMeta::default();
        let json = serde_json::to_vec_pretty(&meta).expect("default SessionMeta serializes");
        if let Err(e) = rekindle_storage::session_meta::save(
            &ctx.paths.session_file, &session_mac_key, &json,
        ) {
            tracing::warn!(error = %e, "failed to write initial session.json");
        }
        meta
    } else {
        match rekindle_storage::session_meta::load(&ctx.paths.session_file, &session_mac_key) {
            Ok(json_bytes) => match serde_json::from_slice(&json_bytes) {
                Ok(meta) => meta,
                Err(e) => {
                    ctx.lifecycle.transition(DaemonState::Locked);
                    return DaemonResponse::error_with_remediation(
                        500, format!("session.json parse failed: {e}"),
                        "session file may be corrupt — re-initialize: rekindle init",
                    );
                }
            },
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return DaemonResponse::error_with_remediation(
                    500, format!("session.json load failed: {e}"),
                    "session file missing or MAC invalid — re-initialize: rekindle init",
                );
            }
        }
    };

    // Step 7: Load transport config
    let transport_config = match load_transport_config(&ctx.paths) {
        Ok(c) => c,
        Err(e) => {
            ctx.lifecycle.transition(DaemonState::Locked);
            return DaemonResponse::error_with_remediation(
                500, format!("transport config failed: {e}"),
                "check config at ~/.config/rekindle/transport.toml",
            );
        }
    };

    // Step 8: Start transport (Veilid attach) — returns (node, inbound_rx)
    let (transport_node, inbound_rx) = match TransportNode::start(transport_config).await {
        Ok(pair) => (Arc::new(pair.0), pair.1),
        Err(e) => {
            ctx.lifecycle.transition(DaemonState::Locked);
            return DaemonResponse::error_with_remediation(
                500, format!("transport start failed: {e}"),
                "check network connectivity and Veilid configuration",
            );
        }
    };

    let mut guard = TransportGuard::new(Arc::clone(&transport_node));

    // Step 9: Wrap in VeilidTransport
    let veilid_transport = Arc::new(VeilidTransport::new(Arc::clone(&transport_node)));
    let transport: Arc<dyn Transport> = veilid_transport;
    let transport_for_ctx = Arc::clone(&transport);

    // Step 10: Construct ChatService
    let chat = match ChatService::new(
        transport, Arc::clone(&vault), session_meta,
        ctx.paths.session_file.clone(), session_mac_key,
    ) {
        Ok(c) => c,
        Err(e) => {
            ctx.lifecycle.transition(DaemonState::Locked);
            return DaemonResponse::error_with_remediation(
                500, format!("chat service init failed: {e}"),
                "vault contents may be corrupt — check logs for detail",
            );
        }
    };

    // Step 11: Start inbound event reader (replaces set_callback)
    chat.start_inbound_loop(inbound_rx);

    if let Err(e) = chat.resume().await {
        if is_fresh {
            tracing::info!("fresh node — skipping resume (no identity yet)");
        } else {
            tracing::warn!(error = %e, "chat resume failed — entering degraded state");
            ctx.lifecycle.transition(DaemonState::Degraded);

            let chat = Arc::new(chat);
            *ctx.chat.write() = Some(Arc::clone(&chat));
            *ctx.transport.write() = Some(Arc::clone(&transport_for_ctx));
            *ctx.vault.write() = Some(Arc::clone(&vault));
            guard.disarm();

            return DaemonResponse::ok(&serde_json::json!({
                "state": "degraded",
                "warning": format!("resume incomplete: {e} — some features may be unavailable"),
            }));
        }
    }

    // Step 12: Wire event delivery through SubscriptionRegistry
    let chat = Arc::new(chat);
    let subs = Arc::clone(&ctx.subscriptions);
    let journal = Arc::clone(&ctx.event_journal);
    let pipeline_tx = chat.pipeline_sender().clone();

    tokio::spawn(async move {
        let mut rx = pipeline_tx.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    journal.append(event.clone());
                    subs.fan_out(&event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "event delivery lagging");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Step 13: Store state in DaemonContext
    *ctx.chat.write() = Some(Arc::clone(&chat));
    *ctx.transport.write() = Some(Arc::clone(&transport_for_ctx));
    *ctx.vault.write() = Some(Arc::clone(&vault));
    guard.disarm();

    // Step 14: Spawn background tasks + transition to Operational
    spawn_background_tasks(&chat);
    ctx.lifecycle.transition(DaemonState::Operational);

    tracing::info!("daemon unlocked — operational");
    DaemonResponse::ok(&serde_json::json!({ "state": "operational" }))
}

/// Spawn all periodic background tasks.
///
/// Tasks use `continue` (not `break`) when `is_operational()` is false.
/// On fresh nodes, the identity doesn't exist yet at spawn time — `break`
/// would kill the task permanently. `continue` keeps the loop alive so
/// the task activates once `rekindle init` creates the identity.
fn spawn_background_tasks(chat: &Arc<ChatService>) {
    tracing::info!("spawning background tasks");

    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            c.trigger_inbox_scan();
        }
    });

    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            let _ = c.heartbeat().await;
        }
    });

    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            c.collect_expired_typers();
        }
    });

    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            c.evict_expired_dedup();
        }
    });

    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            let _ = c.sweep_expired_skipped_keys();
        }
    });

    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            let _ = c.flush_session_meta_if_dirty();
        }
    });

    // Join inbox scan — periodic fallback for unreliable DHT watch notifications.
    tracing::info!("spawning join inbox scan background task (30s interval)");
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        tracing::info!("join inbox scan task started");
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            let processed = c.scan_join_inboxes().await;
            if processed > 0 {
                tracing::info!(processed, "periodic inbox scan: new members approved");
            }
        }
    });

    // Mesh populate — re-reads member registries from DHT, discovers routes,
    // populates gossip meshes. First tick fires immediately (not after 90s)
    // so messages sent in the first 90s have mesh peers to deliver to.
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(90));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Do NOT skip the first tick — first populate must happen immediately.
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            let mesh = c.refresh_community_routes().await;
            if mesh > 0 {
                tracing::debug!(mesh_peers = mesh, "mesh populate cycle complete");
            }
        }
    });

    // Slow-path channel message catch-up — reads DhtLog sentinel (subkey 0)
    // per member per channel to discover messages missed by gossip.
    // Interval scales with community count: max(60s, communities × 6s).
    // At 10 communities: 60s. At 100 communities: 600s (10 min).
    // Sentinel read is O(1) per member per channel — only scans message
    // subkeys when new messages exist.
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }

            // Adaptive interval based on community count
            let community_count = c.community_count();
            let next_secs = 60u64.max(community_count as u64 * 6);
            interval.reset_after(Duration::from_secs(next_secs));

            let caught = c.catch_up_channel_messages().await;
            if caught > 0 {
                tracing::info!(caught, interval_secs = next_secs, "slow-path catch-up: new messages discovered");
            }
        }
    });

    // DM SMPL catch-up — re-establish failed watches and read undelivered
    // DM messages from SMPL records. 30s interval matches the inbox scan
    // cadence. First tick fires immediately so messages sent during the
    // watch-failure window are caught on the first cycle.
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            let caught = c.catch_up_dm_messages().await;
            if caught > 0 {
                tracing::info!(caught, "dm catch-up: messages delivered via poll fallback");
            }
        }
    });

    // MEK auto-rotation
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !c.is_operational() { continue; }
            let rotated = c.check_mek_rotation().await;
            if rotated > 0 {
                tracing::info!(rotated, "MEK auto-rotation cycle complete");
            }
        }
    });
}

// ── Status ─────────────────────────────────────────────────────────────

const STATUS_CACHE_TTL: Duration = Duration::from_secs(1);

pub(crate) fn handle_status(ctx: &Arc<DaemonContext>, state: DaemonState) -> DaemonResponse {
    {
        let cache = ctx.status_cache.lock();
        if let Some((computed_at, ref snapshot)) = *cache {
            if computed_at.elapsed() < STATUS_CACHE_TTL {
                return DaemonResponse::ok(snapshot);
            }
        }
    }

    let chat = ctx.chat.read().clone();
    let counters = ctx.transport_counters.snapshot();

    let snapshot = StatusSnapshot {
        state: state.as_str().to_string(),
        has_identity: chat.as_ref().is_some_and(|c| c.session_identity().is_some()),
        identity_public_key: chat.as_ref().and_then(|c| c.session_identity().map(|id| id.public_key.to_hex())),
        identity_display_name: chat.as_ref().and_then(|c| c.session_identity().map(|id| id.display_name.clone())),
        attachment: chat.as_ref().map_or("unknown".into(), |c| c.io().transport().attachment_state().to_string()),
        is_attached: chat.as_ref().is_some_and(|c| c.io().transport().is_attached()),
        public_internet_ready: chat.as_ref().is_some_and(|c| c.io().transport().is_public_internet_ready()),
        uptime_secs: chat.as_ref().map_or(0, |c| c.io().transport().uptime_secs()),
        peer_count: chat.as_ref().map_or(0, |c| c.io().transport().peer_count() as usize),
        route_allocated: chat.as_ref().is_some_and(|c| c.io().transport().route_blob().is_some()),
        route_age_secs: chat.as_ref().and_then(|c| c.io().transport().route_age_secs()),
        active_watches: chat.as_ref().map_or(0, |c| c.watch_count()),
        gossip_meshes: chat.as_ref().map_or(0, |c| c.community_count()),
        gossip_mesh_peers: chat.as_ref().map_or(0, |c| c.io().transport().gossip_mesh_peer_count()),
        unread_channels: chat.as_ref().map_or(0, |c| c.unread_channels().len()),
        unread_dms: chat.as_ref().map_or(0, |c| c.unread_dms().len()),
        unread_friend_requests: chat.as_ref().map_or(0, |c| c.unread_friend_requests()),
        dedup_entries: chat.as_ref().map_or(0, |c| c.dedup_stats().0),
        dedup_suppressed: chat.as_ref().map_or(0, |c| c.dedup_stats().1),
        poll_loop_active: chat.is_some(),
        renewal_loop_active: chat.is_some(),
        community_count: chat.as_ref().map_or(0, |c| c.community_count()),
        friend_count: chat.as_ref().map_or(0, |c| c.friend_count()),
        circuit_summary: {
            let (total, healthy, degraded, circuit_open) = chat.as_ref()
                .map_or((0, 0, 0, 0), |c| c.io().transport().circuit_summary());
            CircuitSummary { total, healthy, degraded, circuit_open }
        },
        bulk_frames_sent: counters.frames_sent,
        bulk_frames_received: counters.frames_received,
        bulk_bytes_sent: counters.bytes_sent,
        bulk_bytes_received: counters.bytes_received,
        bulk_transfers_active: ctx.bulk_transfers.lock().active_count(),
        checks: build_checks(ctx, state, chat.as_ref().map(AsRef::as_ref)),
    };

    {
        let mut cache = ctx.status_cache.lock();
        *cache = Some((std::time::Instant::now(), snapshot.clone()));
    }

    DaemonResponse::ok(&snapshot)
}

fn build_checks(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    chat: Option<&ChatService>,
) -> Vec<Check> {
    let mut checks = Vec::new();

    checks.push(if state.can_query() {
        Check::pass("node.state", "node", state.as_str())
    } else if state == DaemonState::Locked {
        Check::warn("node.state", "node", state.as_str())
            .with_description("unlock the daemon: rekindle unlock")
    } else {
        Check::fail("node.state", "node", state.as_str())
    });

    let transport_started = ctx.transport.read().is_some();
    checks.push(if transport_started {
        Check::pass("node.transport", "node", "started")
    } else {
        Check::fail("node.transport", "node", "not started")
            .with_description("transport starts during unlock")
    });

    if let Some(chat) = chat {
        let attached = chat.io().transport().is_attached();
        checks.push(if attached {
            Check::pass("transport.attachment", "transport", "attached")
        } else {
            Check::warn("transport.attachment", "transport", "detached")
                .with_description("not attached to Veilid network — check connectivity")
        });

        let peers = chat.io().transport().peer_count();
        checks.push(if peers > 0 {
            Check::pass("transport.peer_count", "transport", peers.to_string())
        } else {
            Check::warn("transport.peer_count", "transport", "0")
                .with_description("no known peers — node may be isolated")
        });

        checks.push(Check::pass("node.uptime", "node", fmt_uptime(chat.io().transport().uptime_secs())));

        checks.push(if chat.io().is_identity_loaded() {
            Check::pass("crypto.identity", "crypto", "loaded")
        } else {
            Check::warn("crypto.identity", "crypto", "not loaded")
                .with_description("identity not in memory — daemon is locked")
        });

        if let Some(identity) = chat.session_identity() {
            checks.push(Check::pass("identity.initialized", "identity", "yes"));
            let pk_short = identity.public_key.display_short();
            checks.push(Check::pass("identity.public_key", "identity", pk_short));
            checks.push(Check::pass("identity.display_name", "identity", &identity.display_name));
        } else {
            checks.push(Check::fail("identity.initialized", "identity", "no")
                .with_description("run: rekindle init"));
        }

        checks.push(Check::pass("subscriptions.watches", "subscriptions", chat.watch_count().to_string()));
    }

    checks.push(if ctx.paths.session_file.exists() {
        Check::pass("storage.session_file", "storage", "exists")
    } else {
        Check::warn("storage.session_file", "storage", "missing")
            .with_description("no session file — identity not initialized")
    });

    checks.push(if ctx.vault.read().is_some() {
        Check::pass("storage.vault", "storage", "open")
    } else {
        Check::warn("storage.vault", "storage", "closed")
    });

    checks.push(Check::pass(
        "subscriptions.connections", "subscriptions",
        ctx.subscriptions.connection_count().to_string(),
    ));

    checks
}

fn fmt_uptime(secs: u64) -> String {
    if secs < 60 { return format!("{secs}s"); }
    let mins = secs / 60;
    if mins < 60 { return format!("{mins}m {}s", secs % 60); }
    let hours = mins / 60;
    if hours < 24 { return format!("{hours}h {}m", mins % 60); }
    let days = hours / 24;
    format!("{days}d {}h", hours % 24)
}

// ── Lock ────────────────────────────────────────────────────────────────

pub(crate) async fn handle_lock(ctx: &Arc<DaemonContext>, state: DaemonState) -> DaemonResponse {
    if !state.can_write() && state != DaemonState::Degraded && state != DaemonState::Detached {
        return DaemonResponse::error(409, format!("cannot lock in state '{}'", state.as_str()));
    }

    ctx.lifecycle.transition(DaemonState::Locking);

    let chat = ctx.chat.read().clone();
    if let Some(ref chat) = chat {
        chat.lock().await;
    }

    let transport_node = ctx.transport.read().clone();
    if let Some(ref node) = transport_node {
        node.shutdown().await.ok();
    }

    *ctx.chat.write() = None;
    *ctx.transport.write() = None;
    *ctx.vault.write() = None;

    ctx.lifecycle.transition(DaemonState::Locked);
    tracing::info!("daemon locked — all secrets zeroized");

    DaemonResponse::ok(&serde_json::json!({ "state": "locked" }))
}

// ── Shutdown ────────────────────────────────────────────────────────────

pub(crate) async fn handle_shutdown(ctx: &Arc<DaemonContext>, state: DaemonState) -> DaemonResponse {
    if state == DaemonState::ShuttingDown {
        return DaemonResponse::ok(&serde_json::json!({ "state": "already_shutting_down" }));
    }

    tracing::info!("shutdown requested via IPC");

    if state.can_query() || state == DaemonState::Resuming {
        let chat = ctx.chat.read().clone();
        if let Some(ref chat) = chat {
            chat.lock().await;
        }
        let transport_node = ctx.transport.read().clone();
        if let Some(ref node) = transport_node {
            node.shutdown().await.ok();
        }
        *ctx.chat.write() = None;
        *ctx.transport.write() = None;
        *ctx.vault.write() = None;
    }

    ctx.lifecycle.transition(DaemonState::ShuttingDown);

    DaemonResponse::ok(&serde_json::json!({
        "state": "shutting_down",
        "message": "daemon will exit after draining connections",
    }))
}

// ── Config loading ──────────────────────────────────────────────────────

pub(crate) fn load_early_config(
    paths: &crate::state::StatePaths,
) -> rekindle_types::config::TransportConfig {
    load_transport_config(paths).unwrap_or_default()
}

fn load_transport_config(
    paths: &crate::state::StatePaths,
) -> Result<rekindle_types::config::TransportConfig, String> {
    let mut config = rekindle_types::config::TransportConfig {
        storage_dir: paths.veilid_dir.display().to_string(),
        namespace: "rekindle".to_string(),
        ..Default::default()
    };

    merge_from_cli_config(&mut config, &std::path::PathBuf::from("/etc/rekindle/config.toml"));
    merge_from_cli_config(&mut config, &paths.config_dir.join("config.toml"));

    let transport_file = paths.config_dir.join("transport.toml");
    if transport_file.exists() {
        match std::fs::read_to_string(&transport_file) {
            Ok(content) => match toml::from_str::<rekindle_types::config::TransportConfig>(&content) {
                Ok(override_config) => {
                    config = override_config;
                    tracing::info!(path = %transport_file.display(), "loaded transport.toml override");
                }
                Err(e) => return Err(format!("parse {}: {e}", transport_file.display())),
            },
            Err(e) => return Err(format!("read {}: {e}", transport_file.display())),
        }
    }

    if let Ok(env_path) = std::env::var("REKINDLE_CONFIG") {
        merge_from_cli_config(&mut config, &std::path::PathBuf::from(env_path));
    }

    config.storage_dir = paths.veilid_dir.display().to_string();
    Ok(config)
}

fn merge_from_cli_config(
    config: &mut rekindle_types::config::TransportConfig,
    path: &std::path::Path,
) {
    #[derive(serde::Deserialize, Default)]
    struct CliConfig {
        #[serde(default)]
        network: NetworkSection,
    }
    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    struct NetworkSection {
        rpc_timeout_ms: Option<u64>,
        dht_write_retries: Option<u32>,
        route_refresh_secs: Option<u64>,
        route_cache_ttl_secs: Option<u64>,
        circuit_breaker_threshold: Option<u32>,
        circuit_breaker_cooldown_secs: Option<u64>,
        dedup_cache_capacity: Option<usize>,
        gossip_ttl: Option<u8>,
        allow_insecure_protected_store: Option<bool>,
        metrics_port: Option<u16>,
        health_port: Option<u16>,
        veilid: Option<rekindle_types::config::VeilidNetworkConfig>,
    }

    let Ok(content) = std::fs::read_to_string(path) else { return };

    let cli_config: CliConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to parse config — skipping");
            return;
        }
    };

    let n = &cli_config.network;
    if let Some(v) = n.rpc_timeout_ms { config.rpc_timeout_ms = v; }
    if let Some(v) = n.dht_write_retries { config.dht_write_retries = v; }
    if let Some(v) = n.route_refresh_secs { config.route_refresh_secs = v; }
    if let Some(v) = n.route_cache_ttl_secs { config.route_cache_ttl_secs = v; }
    if let Some(v) = n.circuit_breaker_threshold { config.circuit_breaker_threshold = v; }
    if let Some(v) = n.circuit_breaker_cooldown_secs { config.circuit_breaker_cooldown_secs = v; }
    if let Some(v) = n.dedup_cache_capacity { config.dedup_cache_capacity = v; }
    if let Some(v) = n.gossip_ttl { config.gossip_ttl = v; }
    if let Some(v) = n.allow_insecure_protected_store { config.allow_insecure_protected_store = v; }
    if let Some(v) = n.metrics_port { config.metrics_port = v; }
    if let Some(v) = n.health_port { config.health_port = v; }
    if let Some(v) = n.veilid.clone() { config.veilid = v; }
}
