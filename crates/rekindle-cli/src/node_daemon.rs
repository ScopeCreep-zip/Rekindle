//! Daemon startup logic for `rekindle node start`.
//!
//! This module contains the full daemon lifecycle: initialize tracing,
//! resolve state paths, generate/load bus keypair, start the Veilid
//! transport node, bind the IPC socket, and run the accept loop.
//!
//! Feature-gated behind `daemon` — only compiled when building the
//! full binary that can run both CLI client and daemon server.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_node::daemon::dispatch::{DaemonContext, PolicyConfig};
use rekindle_node::daemon::handler::DaemonHandler;
use rekindle_node::daemon::{DaemonLifecycle, DaemonState};
use rekindle_node::ipc;
use rekindle_node::state::StatePaths;

use rekindle_transport::crypto::mek::MekCache;

/// Run the daemon in foreground. This is the handler for `rekindle node start`.
///
/// Blocks until SIGTERM/Ctrl-C is received, then performs graceful shutdown.
pub async fn run_daemon(_attach_timeout: u64) -> anyhow::Result<()> {
    // ── 1. Resolve paths and ensure directories ───────────────────
    let paths = StatePaths::resolve()?;
    paths.ensure_directories().await?;
    tracing::info!(
        state_dir = %paths.state_dir.display(),
        veilid_dir = %paths.veilid_dir.display(),
        "state directories ready"
    );

    // ── 2. Initialize daemon lifecycle ────────────────────────────
    let lifecycle = Arc::new(DaemonLifecycle::new());
    let _ = lifecycle.transition(DaemonState::Starting);

    // ── 3. Generate or load bus keypair ───────────────────────────
    let runtime_dir = ipc::runtime_dir()?;
    tokio::fs::create_dir_all(&runtime_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    ipc::noise_keys::create_keys_dir().await?;

    let bus_keypair = load_or_generate_bus_keypair(&runtime_dir).await?;

    // ── 4. Load session from disk ─────────────────────────────────
    let session = rekindle_node::state::load_session(&paths.session_file)?;
    let has_identity = session.is_some();
    tracing::info!(has_identity, "session loaded");

    // ── 5. Build transport config and start Veilid node ───────────
    let transport_config = rekindle_transport::TransportConfig {
        storage_dir: paths.veilid_dir.display().to_string(),
        allow_insecure_protected_store: true, // daemon may run headless
        ..rekindle_transport::TransportConfig::default()
    };

    let session_arc = Arc::new(parking_lot::RwLock::new(session));
    let mek_cache = Arc::new(parking_lot::RwLock::new(MekCache::new()));
    let signing_key_arc: Arc<
        parking_lot::RwLock<Option<rekindle_node::state::keystore::SigningKeyHandle>>,
    > = Arc::new(parking_lot::RwLock::new(None));
    let registry = Arc::new(tokio::sync::RwLock::new(ipc::ClearanceRegistry::new()));

    // Transport SubscriptionManager starts as None — created during unlock/resume.
    // The handler checks subscriptions.read().is_some() before forwarding.
    let transport_subscriptions: Arc<
        parking_lot::RwLock<Option<rekindle_transport::SubscriptionManager>>,
    > = Arc::new(parking_lot::RwLock::new(None));

    // Transport starts as None — filled after TransportNode::start().
    let transport_for_handler: Arc<
        parking_lot::RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    > = Arc::new(parking_lot::RwLock::new(None));

    let pending_joins = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));

    // Departure-triggered MEK rotation. Both triggers (a moderation ban
    // and an inbound leave notification) only need the sender; the
    // worker is spawned below, once the `Arc<DaemonContext>` it needs
    // exists.
    let (mek_rotation_tx, mek_rotation_rx) = rekindle_node::daemon::mek_rotation::channel();

    let handler = Arc::new(DaemonHandler::new(
        Arc::clone(&transport_subscriptions),
        Arc::clone(&session_arc),
        paths.session_file.clone(),
        Arc::clone(&mek_cache),
        Arc::clone(&signing_key_arc),
        Arc::clone(&transport_for_handler),
        Arc::clone(&pending_joins),
        mek_rotation_tx.clone(),
    ));

    let transport = match rekindle_transport::TransportNode::start(
        transport_config,
        handler,
        Arc::clone(&session_arc),
    )
    .await
    {
        Ok(node) => {
            let arc = Arc::new(node);
            // Fill the handler's transport reference now that it's started.
            *transport_for_handler.write() = Some(Arc::clone(&arc));
            tracing::info!("transport node started");
            Some(arc)
        }
        Err(e) => {
            tracing::warn!(error = %e, "transport node start failed — running without network");
            None
        }
    };

    // ── 6. Create DaemonContext ───────────────────────────────────
    let audit_logger = match rekindle_node::state::audit::AuditLogger::open(
        &paths.state_dir.join("audit.jsonl"),
    ) {
        Ok(logger) => {
            tracing::info!(sequence = logger.sequence(), "audit logger initialized");
            Some(logger)
        }
        Err(e) => {
            tracing::warn!(error = %e, "audit logger init failed — continuing without audit");
            None
        }
    };

    // Watch channel for event source notification.
    // handle_unlock sends the broadcast::Sender through this when SubscriptionManager
    // is created. The IPC server receives it and starts its internal delivery task.
    let (event_watch_tx, event_watch_rx) = tokio::sync::watch::channel(None);

    let daemon_ctx = Arc::new(DaemonContext {
        community_runtime: Arc::new(
            rekindle_node::daemon::community_runtime::CommunityRuntimeMap::new(),
        ),
        transport: RwLock::new(transport),
        session: session_arc,
        mek_cache,
        signing_key: signing_key_arc,
        lifecycle: Arc::clone(&lifecycle),
        session_path: paths.session_file.clone(),
        registry: Arc::clone(&registry),
        policy: RwLock::new(PolicyConfig::default()),
        config_dir: paths.config_dir.clone(),
        audit: parking_lot::Mutex::new(audit_logger),
        subscriptions: RwLock::new(None),
        broadcast_mgr: RwLock::new(None),
        event_watch_tx,
        pending_joins: Arc::clone(&pending_joins),
        mek_rotation_tx,
    });

    // The rotation worker owns an `Arc<DaemonContext>` — the reason the
    // triggers queue instead of spawning. It waits out the cascade
    // (tens of seconds) off the request path.
    let mek_rotation_worker = tokio::spawn(rekindle_node::daemon::mek_rotation::run_worker(
        Arc::clone(&daemon_ctx),
        mek_rotation_rx,
    ));

    // ── 7. Bind IPC socket and create bus server ──────────────────
    let socket_path = ipc::socket_path()?;

    // Generate the daemon subscriber's keypair BEFORE binding the server.
    // This keypair is registered in the clearance registry so the server
    // authenticates the subscriber at Internal level when it connects.
    let daemon_subscriber_kp = ipc::generate_keypair()
        .map_err(|e| anyhow::anyhow!("daemon subscriber keypair generation failed: {e}"))?;
    let daemon_subscriber_pubkey: [u8; 32] = daemon_subscriber_kp
        .as_inner()
        .public
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("subscriber pubkey is not 32 bytes"))?;

    let mut clearance_registry = ipc::ClearanceRegistry::new();
    clearance_registry.register(
        rekindle_node::ipc::server::DAEMON_AGENT_NAME.to_string(),
        daemon_subscriber_pubkey,
        ipc::SecurityLevel::Internal,
        ipc::AgentType::System,
        vec!["dispatch".into()],
    );

    let bus_server =
        ipc::BusServer::bind(&socket_path, bus_keypair.into_inner(), clearance_registry)?;
    tracing::info!(path = %socket_path.display(), "IPC bus server bound");

    // Start the event delivery system. The delivery task awaits the watch
    // channel for the broadcast sender (populated during handle_unlock).
    // Events route in-process through the EventRouter — no bridge task needed.
    bus_server.start_event_delivery(event_watch_rx);

    // ── 8. Transition to Locked ───────────────────────────────────
    let _ = lifecycle.transition(DaemonState::Locked);
    tracing::info!(
        state = lifecycle.state().as_str(),
        "daemon accepting connections"
    );

    // ── 9. Notify systemd READY=1 (Type=notify) ─────────────────
    notify_ready();

    // ── 9b. Spawn daemon bus subscriber ──────────────────────────
    // The daemon connects to its own socket as a privileged internal
    // agent using the keypair registered in the clearance registry.
    // Requests are unicast to this connection by the server.
    let subscriber_handle = spawn_bus_subscriber(
        Arc::clone(&daemon_ctx),
        socket_path.clone(),
        daemon_subscriber_kp,
    );

    // Event delivery is handled in-process by BusServer::start_event_delivery().
    // No bridge task needed — the server's internal task subscribes directly to
    // the SubscriptionManager's broadcast channel when handle_unlock notifies via
    // the watch channel. Three-tier event delivery (watch + gossip + poll) is
    // set up during handle_unlock when SubscriptionManager is created.

    // ── 9d. Spawn daemon-internal event consumer ─────────────────────
    // Subscribes to SubscriptionManager events and triggers daemon-internal
    // actions (process_inbox, friend inbox scan) when tier 3 poll discovers
    // changes that tier 1 watch missed. Completes the three-tier guarantee.
    let consumer_handle = spawn_event_consumer(Arc::clone(&daemon_ctx), Arc::clone(&lifecycle));

    // ── 10. Run IPC accept loop with shutdown signal + watchdog ───
    let mut watchdog_interval = tokio::time::interval(std::time::Duration::from_secs(15));
    tokio::select! {
        result = bus_server.run() => {
            if let Err(e) = result {
                tracing::error!(error = %e, "bus server fatal error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutdown signal received (Ctrl+C)");
        }
        () = lifecycle.shutdown_requested() => {
            tracing::info!("shutdown requested via IPC");
        }
        () = async {
            loop {
                watchdog_interval.tick().await;
                notify_watchdog();
                tracing::trace!(
                    state = lifecycle.state().as_str(),
                    "watchdog ping"
                );
            }
        } => {
            unreachable!("watchdog loop does not terminate");
        }
    }

    // ── 11. Graceful shutdown ─────────────────────────────────────
    let _ = lifecycle.transition(DaemonState::ShuttingDown);
    tracing::info!("draining connections...");

    // Drop the bus server first — closes the socket, disconnects all clients
    // including the daemon's own subscriber. The subscriber task will exit
    // when its connection drops.
    drop(bus_server);
    subscriber_handle.abort();
    consumer_handle.abort();
    // Aborted rather than drained: a rotation in flight is mid-cascade
    // and may have tens of seconds left, and an unrotated key is the
    // same outcome as a rotation nobody was online to receive — which
    // the protocol already handles by rotating when someone returns.
    mek_rotation_worker.abort();
    let _ = subscriber_handle.await;
    let _ = consumer_handle.await;
    let _ = mek_rotation_worker.await;

    shutdown_transport(&daemon_ctx).await;

    *daemon_ctx.signing_key.write() = None;

    let _ = lifecycle.transition(DaemonState::Stopped);
    tracing::info!("rekindle daemon stopped");

    Ok(())
}

/// Spawn the daemon's own bus subscriber.
///
/// The daemon connects to its own socket as a privileged internal agent
/// using the keypair registered in the clearance registry. Requests are
/// unicast to this connection by the server.
fn spawn_bus_subscriber(
    daemon_ctx: Arc<DaemonContext>,
    subscriber_socket: std::path::PathBuf,
    daemon_subscriber_kp: ipc::ZeroizingKeypair,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Small delay to let the accept loop start.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let server_pub = match ipc::noise_keys::read_bus_public_key().await {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(error = %e, "daemon subscriber: cannot read bus public key");
                return;
            }
        };

        let sender_id = uuid::Uuid::now_v7();
        let client = match ipc::BusClient::connect_with_retry(
            sender_id,
            &subscriber_socket,
            &server_pub,
            daemon_subscriber_kp.as_inner(),
            5,
            std::time::Duration::from_millis(200),
        )
        .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "daemon subscriber: failed to connect to own socket");
                return;
            }
        };

        tracing::info!("daemon bus subscriber connected");
        daemon_ctx.run_subscriber(client).await;
    })
}

/// Gracefully shut down the transport node, if one is running.
async fn shutdown_transport(daemon_ctx: &Arc<DaemonContext>) {
    let transport = daemon_ctx.transport.write().take();
    if let Some(node) = transport {
        match Arc::try_unwrap(node) {
            Ok(n) => {
                if let Err(e) = n.shutdown().await {
                    tracing::warn!(error = %e, "transport shutdown error");
                }
            }
            Err(arc) => {
                tracing::warn!(
                    refs = Arc::strong_count(&arc),
                    "transport shutdown with outstanding references — dropping"
                );
                drop(arc);
            }
        }
    }
}

/// Spawn the daemon-internal tier-3 event consumer.
///
/// Subscribes to `SubscriptionManager` events and triggers daemon-internal
/// actions (process_inbox, friend inbox scan) when tier 3 poll discovers
/// changes that tier 1 watch missed. Completes the three-tier guarantee.
fn spawn_event_consumer(
    daemon_ctx: Arc<DaemonContext>,
    lifecycle: Arc<DaemonLifecycle>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Wait for operational state (SubscriptionManager created during unlock)
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if lifecycle.state() == DaemonState::Operational {
                break;
            }
            if matches!(
                lifecycle.state(),
                DaemonState::ShuttingDown | DaemonState::Stopped
            ) {
                return;
            }
        }

        let mut event_rx = {
            let guard = daemon_ctx.subscriptions.read();
            let Some(ref sub_mgr) = *guard else {
                return;
            };
            sub_mgr.subscribe()
        };

        tracing::info!("daemon-internal event consumer started (tier 3 → process_inbox)");

        loop {
            match event_rx.recv().await {
                Ok(rekindle_types::subscription_events::SubscriptionEvent::Network(
                    rekindle_types::subscription_events::NetworkEvent::ValueChanged {
                        ref record_key,
                        ..
                    },
                )) => {
                    handle_value_changed(&daemon_ctx, record_key).await;
                }
                Ok(_) => {} // Other events — not our concern
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "daemon event consumer: lagging");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!("daemon event consumer: subscription closed");
                    break;
                }
            }
        }
    })
}

/// Handle a tier-3 DHT `ValueChanged` notification by routing it to the
/// matching operator community inbox and/or our friend inbox.
async fn handle_value_changed(daemon_ctx: &Arc<DaemonContext>, record_key: &str) {
    // Check if this is a join inbox for an operator community
    // Check if this is our friend inbox
    let friend_inbox_key = {
        let guard = daemon_ctx.session.read();
        guard.as_ref().and_then(|s| {
            if !s.identity.friend_inbox_key.is_empty() && s.identity.friend_inbox_key == *record_key
            {
                Some(s.identity.friend_inbox_key.clone())
            } else {
                None
            }
        })
    };
    if let Some(inbox_key) = friend_inbox_key {
        tracing::info!("tier 3 poll triggered friend inbox scan");
        rekindle_node::daemon::friend_inbox::scan_friend_inbox(
            &daemon_ctx.session,
            &daemon_ctx.transport,
            &daemon_ctx.session_path,
            &inbox_key,
        )
        .await;
    }
}

/// Load the bus server keypair from disk, or generate a fresh one.
async fn load_or_generate_bus_keypair(
    runtime_dir: &std::path::Path,
) -> anyhow::Result<ipc::ZeroizingKeypair> {
    let pub_path = runtime_dir.join("bus.pub");
    let key_path = runtime_dir.join("bus.key");

    if pub_path.exists() && key_path.exists() {
        match load_bus_keypair(&pub_path, &key_path).await {
            Ok(kp) => {
                tracing::info!("loaded existing bus keypair");
                return Ok(kp);
            }
            Err(e) => {
                tracing::warn!(error = %e, "bus keypair load failed, generating fresh");
            }
        }
    }

    let kp = ipc::generate_keypair()?;
    ipc::noise_keys::write_bus_keypair(kp.as_inner()).await?;
    tracing::info!("bus keypair generated");
    Ok(kp)
}

/// Load the bus server keypair from disk with tamper detection.
async fn load_bus_keypair(
    pub_path: &std::path::Path,
    key_path: &std::path::Path,
) -> anyhow::Result<ipc::ZeroizingKeypair> {
    let public_bytes = tokio::fs::read(pub_path).await?;
    let mut private_bytes = tokio::fs::read(key_path).await?;

    if public_bytes.len() != 32 {
        anyhow::bail!("bus.pub: expected 32 bytes, got {}", public_bytes.len());
    }
    if private_bytes.len() != 32 {
        zeroize::Zeroize::zeroize(&mut private_bytes);
        anyhow::bail!("bus.key: expected 32 bytes, got {}", private_bytes.len());
    }

    let checksum_path = pub_path.with_file_name("bus.checksum");
    if checksum_path.exists() {
        let stored = tokio::fs::read(&checksum_path).await?;
        let pub_array: [u8; 32] = public_bytes
            .clone()
            .try_into()
            .map_err(|_| anyhow::anyhow!("bus.pub not 32 bytes"))?;
        let expected = blake3::keyed_hash(&pub_array, &private_bytes);

        if stored.len() != 32 || !constant_time_eq(&stored, expected.as_bytes()) {
            zeroize::Zeroize::zeroize(&mut private_bytes);
            anyhow::bail!("TAMPER DETECTED: bus keypair checksum mismatch");
        }
    }

    let kp = snow::Keypair {
        private: private_bytes,
        public: public_bytes,
    };
    Ok(ipc::ZeroizingKeypair::new(kp))
}

fn notify_ready() {
    match sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        Ok(()) => tracing::info!("sd_notify: READY=1 sent"),
        Err(e) => tracing::debug!(error = %e, "sd_notify: READY=1 failed (not under systemd?)"),
    }
}

fn notify_watchdog() {
    if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]) {
        tracing::trace!(error = %e, "sd_notify: watchdog ping failed");
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false; // Length mismatch is not timing-sensitive (public info)
    }
    a.ct_eq(b).into()
}
