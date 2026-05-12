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
use rekindle_types::display::{Check, CircuitSummary, StatusSnapshot};
use rekindle_types::session_types::SessionMeta;
use rekindle_types::transport::Transport;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use super::DaemonContext;

// ── Transport cleanup guard ────────────────────────────────────────────
//
// Ensures the transport node is shut down if unlock fails partway through.
// If ChatService construction, callback install, or resume fails after the
// transport has already started, the guard spawns a shutdown task on drop.
// Disarm after successful storage in DaemonContext.

struct TransportGuard {
    node: Option<Arc<TransportNode>>,
}

impl TransportGuard {
    fn new(node: Arc<TransportNode>) -> Self {
        Self { node: Some(node) }
    }

    /// Disarm the guard — transport ownership transferred to DaemonContext.
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

/// Handle Unlock — transition from Locked → Resuming → Operational.
///
/// Linear 15-step flow:
/// 1.  Guard state
/// 2.  Transition to Resuming
/// 3.  Derive master key from passphrase (Argon2id ~500ms)
/// 4.  Open vault (SQLCipher)
/// 5.  Derive session MAC key
/// 6.  Load session.json (verify BLAKE3 MAC)
/// 7.  Load transport config
/// 8.  Start transport (Veilid attach, wait for network)
/// 9.  Wrap transport in VeilidTransport (Transport trait impl)
/// 10. Construct ChatService (loads signing key, sessions, MEKs, friend names)
/// 11. Install TransportCallback (drains buffered events)
/// 12. Resume (open DHT records, publish route, set up watches, join meshes)
/// 13. Wire IPC event delivery
/// 14. Store state in DaemonContext
/// 15. Spawn background tasks, transition to Operational
pub(crate) async fn handle_unlock(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    passphrase: &str,
) -> IpcResponse {
    // Step 1: Guard
    if !state.can_unlock() {
        return IpcResponse::error(
            409,
            format!("cannot unlock in state '{}' — daemon must be locked", state.as_str()),
        );
    }

    // Step 2: Transition
    ctx.lifecycle.transition(DaemonState::Resuming);

    // Step 3: Derive master key from passphrase
    //
    // Fresh node detection: if vault.salt doesn't exist, this is a first-time
    // setup. Generate a random master key, enroll the passphrase (creates
    // vault.salt + vault.wrapped), then proceed with the normal unlock flow.
    // This allows `rekindle init` to call Unlock → IdentityCreate in sequence
    // without requiring a separate "enroll" IPC command.
    let passphrase_unlock = PassphraseUnlock::new(&ctx.paths.state_dir, passphrase.as_bytes());
    // A node is "fresh" if vault.db doesn't exist. vault.db is the last artifact
    // created during a successful first-time unlock (after enrollment + vault create).
    // Checking vault.salt alone is insufficient: a crashed first enrollment may
    // leave vault.salt + vault.wrapped without vault.db. Re-enrolling is safe —
    // enroll() overwrites existing salt/wrapped atomically.
    let is_fresh = !ctx.paths.vault_db.exists();

    let master_key = if is_fresh {
        // First-time enrollment: generate master key, wrap with passphrase
        let mk = match rekindle_storage::unlock::MasterKey::generate() {
            Ok(mk) => mk,
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return IpcResponse::error(500, format!("master key generation failed: {e}"));
            }
        };
        if let Err(e) = passphrase_unlock.enroll(&mk) {
            ctx.lifecycle.transition(DaemonState::Locked);
            return IpcResponse::error(500, format!("passphrase enrollment failed: {e}"));
        }
        tracing::info!("fresh node — vault credentials enrolled");
        mk
    } else {
        match passphrase_unlock.unlock() {
            Ok(mk) => mk,
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return IpcResponse::error_with_remediation(
                    401,
                    format!("unlock failed: {e}"),
                    "check passphrase, or if identity was never initialized: rekindle init",
                );
            }
        }
    };

    // Step 4: Open vault (create on fresh node)
    let vault = if is_fresh {
        match VaultStore::create(&ctx.paths.vault_db, master_key.as_bytes()) {
            Ok(v) => Arc::new(v),
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return IpcResponse::error(500, format!("vault creation failed: {e}"));
            }
        }
    } else {
        match VaultStore::open(&ctx.paths.vault_db, master_key.as_bytes()) {
            Ok(v) => Arc::new(v),
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return IpcResponse::error_with_remediation(
                    500,
                    format!("vault open failed: {e}"),
                    "vault may be corrupt — try: rekindle vault repair",
                );
            }
        }
    };

    // Step 5: Derive session MAC key (for session.json integrity verification)
    let session_mac_key = rekindle_storage::session_meta::derive_mac_key(master_key.as_bytes());

    // Step 6: Load session.json (or create default on fresh node)
    let session_meta: SessionMeta = if is_fresh {
        // Fresh node — no session.json yet. Create empty SessionMeta.
        // IdentityCreate will populate it after unlock completes.
        let meta = SessionMeta::default();
        let json = serde_json::to_vec_pretty(&meta).expect("default SessionMeta serializes");
        if let Err(e) = rekindle_storage::session_meta::save(
            &ctx.paths.session_file, &session_mac_key, &json,
        ) {
            tracing::warn!(error = %e, "failed to write initial session.json — will retry on flush");
        }
        meta
    } else {
        match rekindle_storage::session_meta::load(
            &ctx.paths.session_file,
            &session_mac_key,
        ) {
            Ok(json_bytes) => match serde_json::from_slice(&json_bytes) {
                Ok(meta) => meta,
                Err(e) => {
                    ctx.lifecycle.transition(DaemonState::Locked);
                    return IpcResponse::error_with_remediation(
                        500,
                        format!("session.json parse failed: {e}"),
                        "session file may be corrupt — re-initialize: rekindle init",
                    );
                }
            },
            Err(e) => {
                ctx.lifecycle.transition(DaemonState::Locked);
                return IpcResponse::error_with_remediation(
                    500,
                    format!("session.json load failed: {e}"),
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
            return IpcResponse::error_with_remediation(
                500,
                format!("transport config failed: {e}"),
                "check config at ~/.config/rekindle/transport.toml",
            );
        }
    };

    // Step 8: Start transport (Veilid attach)
    let transport_node = match TransportNode::start(transport_config).await {
        Ok(n) => Arc::new(n),
        Err(e) => {
            ctx.lifecycle.transition(DaemonState::Locked);
            return IpcResponse::error_with_remediation(
                500,
                format!("transport start failed: {e}"),
                "check network connectivity and Veilid configuration",
            );
        }
    };

    // Arm the transport guard — if anything below fails, transport is shut down.
    let mut guard = TransportGuard::new(Arc::clone(&transport_node));

    // Step 9: Wrap in VeilidTransport (Transport trait impl)
    let veilid_transport = Arc::new(VeilidTransport::new(Arc::clone(&transport_node)));
    let transport: Arc<dyn Transport> = veilid_transport;
    let transport_for_ctx = Arc::clone(&transport);

    // Step 10: Construct ChatService
    let chat = match ChatService::new(
        transport,
        Arc::clone(&vault),
        session_meta,
        ctx.paths.session_file.clone(),
        session_mac_key,
    ) {
        Ok(c) => c,
        Err(e) => {
            ctx.lifecycle.transition(DaemonState::Locked);
            return IpcResponse::error_with_remediation(
                500,
                format!("chat service init failed: {e}"),
                "vault contents may be corrupt — check logs for detail",
            );
        }
    };

    // Step 11: Install TransportCallback (drains buffered events)
    transport_node.set_callback(chat.callback());

    // Step 12: Resume (open DHT records, publish route, set up watches, join meshes)
    //
    // Fresh nodes have no identity, no signing key, no DHT records — resume()
    // will fail with NotInitialized. This is expected: the node goes to
    // Operational so `rekindle init` can call IdentityCreate to bootstrap.
    // Existing nodes that fail resume enter Degraded for auto-recovery.
    if let Err(e) = chat.resume().await {
        if is_fresh {
            tracing::info!("fresh node — skipping resume (no identity yet)");
        } else {
            tracing::warn!(error = %e, "chat resume failed — entering degraded state");
            // Don't fail completely — the daemon is partially functional.
            // Transport is running, vault is open, but DHT records may not be open.
            ctx.lifecycle.transition(DaemonState::Degraded);

            // Still store state so Lock/Shutdown can clean up.
            let chat = Arc::new(chat);
            *ctx.chat.write() = Some(Arc::clone(&chat));
            *ctx.transport.write() = Some(Arc::clone(&transport_for_ctx));
            *ctx.vault.write() = Some(Arc::clone(&vault));
            guard.disarm();

            return IpcResponse::ok(&serde_json::json!({
                "state": "degraded",
                "warning": format!("resume incomplete: {e} — some features may be unavailable"),
            }));
        }
    }

    // Step 13: Wire IPC event delivery
    let chat = Arc::new(chat);
    let _ = ctx.event_watch_tx.send(Some(chat.pipeline_sender().clone()));

    // Step 14: Store state in DaemonContext
    *ctx.chat.write() = Some(Arc::clone(&chat));
    *ctx.transport.write() = Some(Arc::clone(&transport_for_ctx));
    *ctx.vault.write() = Some(Arc::clone(&vault));
    guard.disarm();

    // Step 15: Spawn background tasks + transition to Operational
    spawn_background_tasks(&chat);
    ctx.lifecycle.transition(DaemonState::Operational);

    tracing::info!("daemon unlocked — operational");
    IpcResponse::ok(&serde_json::json!({ "state": "operational" }))
}

/// Spawn all periodic background tasks.
///
/// Each task clones the Arc<ChatService> and loops until
/// `chat.is_operational()` returns false (signing key cleared during lock).
fn spawn_background_tasks(chat: &Arc<ChatService>) {
    // Inbox scan (30s) — discovers friend requests and acceptances
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if !c.is_operational() { break; }
            c.trigger_inbox_scan();
        }
    });

    // Heartbeat (60s) — publish online presence
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if !c.is_operational() { break; }
            let _ = c.heartbeat().await;
        }
    });

    // Typing expiry collection (2s) — emit TypingStopped for expired indicators
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            if !c.is_operational() { break; }
            c.collect_expired_typers();
        }
    });

    // Dedup eviction (300s) — clear expired dedup entries
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            if !c.is_operational() { break; }
            c.evict_expired_dedup();
        }
    });

    // Skipped key sweep (3600s) — delete expired skipped message keys from vault
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if !c.is_operational() { break; }
            let _ = c.sweep_expired_skipped_keys();
        }
    });

    // Session.json dirty flush (5s) — persist session_meta if modified
    let c = Arc::clone(chat);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if !c.is_operational() { break; }
            let _ = c.flush_session_meta_if_dirty();
        }
    });
}

// ── Status ─────────────────────────────────────────────────────────────

/// Handle Status — always available, any state.
///
/// Queries ChatService for application-level health when available.
/// Queries DaemonContext for daemon-level health always.
pub(crate) fn handle_status(ctx: &Arc<DaemonContext>, state: DaemonState) -> IpcResponse {
    let chat = ctx.chat.read().clone();

    let snapshot = StatusSnapshot {
        state: state.as_str().to_string(),
        has_identity: chat.as_ref().is_some_and(|c| {
            c.session_identity().is_some()
        }),
        identity_public_key: chat.as_ref().and_then(|c| {
            c.session_identity().map(|id| id.public_key_hex.clone())
        }),
        identity_display_name: chat.as_ref().and_then(|c| {
            c.session_identity().map(|id| id.display_name.clone())
        }),
        attachment: chat.as_ref()
            .map_or("unknown".into(), |c| c.io().transport().attachment_state().to_string()),
        is_attached: chat.as_ref().is_some_and(|c| c.io().transport().is_attached()),
        public_internet_ready: false, // available via transport snapshot if needed
        uptime_secs: chat.as_ref()
            .map_or(0, |c| c.io().transport().uptime_secs()),
        peer_count: chat.as_ref()
            .map_or(0, |c| c.io().transport().peer_count() as usize),  // Transport returns u32, StatusSnapshot needs usize
        route_allocated: false, // available via transport snapshot if needed
        route_age_secs: None,
        active_watches: chat.as_ref().map_or(0, |c| c.watch_count()),
        gossip_meshes: 0,
        gossip_mesh_peers: 0,
        unread_channels: chat.as_ref().map_or(0, |c| c.unread_channels().len()),
        unread_dms: chat.as_ref().map_or(0, |c| c.unread_dms().len()),
        unread_friend_requests: chat.as_ref().map_or(0, |c| c.unread_friend_requests()),
        dedup_entries: 0,
        dedup_suppressed: 0,
        poll_loop_active: chat.is_some(),
        renewal_loop_active: chat.is_some(),
        community_count: chat.as_ref().map_or(0, |c| c.community_count()),
        friend_count: chat.as_ref().map_or(0, |c| c.friend_count()),
        circuit_summary: CircuitSummary {
            total: 0, healthy: 0, degraded: 0, circuit_open: 0,
        },
        checks: build_checks(ctx, state, chat.as_ref().map(AsRef::as_ref)),
    };

    IpcResponse::ok(&snapshot)
}

/// Build diagnostic checks from ChatService API + daemon state.
fn build_checks(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    chat: Option<&ChatService>,
) -> Vec<Check> {
    let mut checks = Vec::new();

    // ── NODE ────────────────────────────────────────────────────────
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

    // ── TRANSPORT ───────────────────────────────────────────────────
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

        let uptime = chat.io().transport().uptime_secs();
        checks.push(Check::pass("node.uptime", "node", fmt_uptime(uptime)));
    }

    // ── CRYPTO ──────────────────────────────────────────────────────
    if let Some(chat) = chat {
        checks.push(if chat.io().is_signing_key_loaded() {
            Check::pass("crypto.signing_key", "crypto", "loaded")
        } else {
            Check::warn("crypto.signing_key", "crypto", "not loaded")
                .with_description("signing key not in memory — daemon is locked")
        });
    }

    // ── STORAGE ─────────────────────────────────────────────────────
    checks.push(if ctx.paths.session_file.exists() {
        Check::pass("storage.session_file", "storage", "exists")
    } else {
        Check::warn("storage.session_file", "storage", "missing")
            .with_description("no session file — identity not initialized")
    });

    let vault_open = ctx.vault.read().is_some();
    checks.push(if vault_open {
        Check::pass("storage.vault", "storage", "open")
    } else {
        Check::warn("storage.vault", "storage", "closed")
    });

    // ── IDENTITY ────────────────────────────────────────────────────
    if let Some(chat) = chat {
        if let Some(identity) = chat.session_identity() {
            checks.push(Check::pass("identity.initialized", "identity", "yes"));
            let pk = &identity.public_key_hex;
            let pk_short = if pk.len() > 16 {
                format!("{}...{}", &pk[..8], &pk[pk.len() - 4..])
            } else {
                pk.clone()
            };
            checks.push(Check::pass("identity.public_key", "identity", pk_short));
            checks.push(Check::pass(
                "identity.display_name", "identity", &identity.display_name,
            ));
        } else {
            checks.push(Check::fail("identity.initialized", "identity", "no")
                .with_description("run: rekindle init"));
        }
    }

    // ── SUBSCRIPTIONS ───────────────────────────────────────────────
    if let Some(chat) = chat {
        checks.push(Check::pass(
            "subscriptions.watches", "subscriptions",
            chat.watch_count().to_string(),
        ));
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

// ── Lock ────────────────────────────────────────────────────────────────

/// Handle Lock — transition to Locked, zeroize all secrets.
///
/// Order: transition Locking → chat.lock() → transport shutdown →
/// clear references → transition Locked.
///
/// chat.lock() cancels watches, clears session cache, clears MEK cache,
/// clears signing key, flushes dirty session.json. After lock(), the
/// ChatService is inert — is_operational() returns false, background
/// tasks exit their loops.
pub(crate) async fn handle_lock(ctx: &Arc<DaemonContext>, state: DaemonState) -> IpcResponse {
    if !state.can_write() && state != DaemonState::Degraded && state != DaemonState::Detached {
        return IpcResponse::error(
            409,
            format!("cannot lock in state '{}'", state.as_str()),
        );
    }

    ctx.lifecycle.transition(DaemonState::Locking);

    // Lock ChatService (zeroize secrets, cancel watches, flush session.json)
    let chat = ctx.chat.read().clone();
    if let Some(ref chat) = chat {
        chat.lock().await;
    }

    // Shutdown transport
    let transport_node = ctx.transport.read().clone();
    if let Some(ref node) = transport_node {
        node.shutdown().await.ok();
    }

    // Clear references — last Arc holders trigger Drop cleanup.
    // VaultStore::drop does PRAGMA rekey = '' (best-effort C buffer clear).
    *ctx.chat.write() = None;
    *ctx.transport.write() = None;
    *ctx.vault.write() = None;

    ctx.lifecycle.transition(DaemonState::Locked);
    tracing::info!("daemon locked — all secrets zeroized");

    IpcResponse::ok(&serde_json::json!({ "state": "locked" }))
}

// ── Shutdown ────────────────────────────────────────────────────────────

/// Handle Shutdown — initiate graceful daemon shutdown.
///
/// Performs lock first (zeroize secrets, stop transport), then transitions
/// to ShuttingDown which notifies the main event loop to exit.
pub(crate) async fn handle_shutdown(ctx: &Arc<DaemonContext>, state: DaemonState) -> IpcResponse {
    if state == DaemonState::ShuttingDown {
        return IpcResponse::ok(&serde_json::json!({ "state": "already_shutting_down" }));
    }

    tracing::info!("shutdown requested via IPC");

    // If operational/degraded/detached, perform lock first.
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

    // Transition to ShuttingDown — notifies the main event loop via DaemonLifecycle.
    ctx.lifecycle.transition(DaemonState::ShuttingDown);

    IpcResponse::ok(&serde_json::json!({
        "state": "shutting_down",
        "message": "daemon will exit after draining connections",
    }))
}

// ── Config loading ──────────────────────────────────────────────────────

/// Load only the ports from config at daemon startup (before unlock).
///
/// Used by `run_daemon()` to bind metrics and health endpoints on the
/// configured ports before the first unlock. Falls back to defaults.
pub(crate) fn load_early_config(
    paths: &crate::state::StatePaths,
) -> rekindle_types::config::TransportConfig {
    load_transport_config(paths).unwrap_or_default()
}

/// Load transport configuration from all standard config paths.
///
/// Reads the same config files the CLI reads, in the same precedence order:
/// 1. /etc/rekindle/config.toml (system-wide, lowest priority)
/// 2. ~/.config/rekindle/config.toml (user)
/// 3. ~/.config/rekindle/transport.toml (daemon-specific override)
/// 4. REKINDLE_CONFIG env var
///
/// The `[network]` section maps directly to TransportConfig fields.
/// Falls back to defaults if no config files exist.
fn load_transport_config(
    paths: &crate::state::StatePaths,
) -> Result<rekindle_types::config::TransportConfig, String> {
    let mut config = rekindle_types::config::TransportConfig {
        storage_dir: paths.veilid_dir.display().to_string(),
        namespace: "rekindle".to_string(),
        ..Default::default()
    };

    // Layer 1: system-wide config (/etc/rekindle/config.toml)
    // Uses the CLI config format with [network] section — extract transport fields.
    merge_from_cli_config(&mut config, &std::path::PathBuf::from("/etc/rekindle/config.toml"));

    // Layer 2: user config (~/.config/rekindle/config.toml)
    merge_from_cli_config(&mut config, &paths.config_dir.join("config.toml"));

    // Layer 3: daemon-specific transport override (~/.config/rekindle/transport.toml)
    // This file uses TransportConfig format directly (no [network] wrapper).
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

    // Layer 4: env var override
    if let Ok(env_path) = std::env::var("REKINDLE_CONFIG") {
        merge_from_cli_config(&mut config, &std::path::PathBuf::from(env_path));
    }

    // Always enforce XDG storage dir — config files cannot override it.
    config.storage_dir = paths.veilid_dir.display().to_string();

    tracing::debug!(
        allow_insecure_protected_store = config.allow_insecure_protected_store,
        namespace = %config.namespace,
        "transport config resolved"
    );

    Ok(config)
}

/// Extract transport-relevant fields from a CLI-format config file.
///
/// The CLI config has `[network]` section with fields that map 1:1 to
/// TransportConfig. This function reads the file, parses the [network]
/// section, and merges non-default values into the transport config.
fn merge_from_cli_config(
    config: &mut rekindle_types::config::TransportConfig,
    path: &std::path::Path,
) {
    /// Minimal CLI config shape — just enough to extract [network].
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

    tracing::debug!(path = %path.display(), "merged config layer");
}
