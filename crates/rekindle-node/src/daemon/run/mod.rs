//! Daemon process runtime — orchestrates all OS-level concerns.
//!
//! `run_daemon()` is the process entry point. It owns:
//! - Process hardening (core dump suppression, non-dumpable, resource limits)
//! - PID file lifecycle (creation, stale detection, cleanup)
//! - IPC bus (socket bind, accept loop, event delivery)
//! - Daemon subscriber (connects to own socket for request dispatch)
//! - Signal handling (SIGTERM, SIGHUP, SIGUSR1, SIGUSR2)
//! - systemd integration (READY=1, WATCHDOG=1, STATUS=)
//! - Sandbox application (Landlock + seccomp after socket bind)
//! - Config hot-reload (inotify/kqueue watch on config directory)
//! - Metrics emission (Prometheus /metrics endpoint on localhost)
//! - Health check (lightweight TCP probe endpoint)
//! - Graceful shutdown (ordered drain, timeout, force-kill)
//!
//! The daemon starts in LOCKED state. All business logic (transport start,
//! vault open, ChatService construction) happens inside `handle_unlock`
//! when the user runs `rekindle unlock` via IPC.

pub mod signals;
pub mod pid;
pub mod sandbox;
pub mod hardening;
pub mod systemd;
pub mod config_watch;
pub mod metrics;
pub mod health;
pub mod tracing_init;

use std::sync::Arc;
use std::time::Duration;

use crate::daemon::{DaemonLifecycle, DaemonState};
use crate::daemon::dispatch::DaemonContext;
use crate::ipc;
use crate::state::StatePaths;

/// Run the rekindle daemon. This is the sole process entry point.
///
/// Blocks until shutdown signal (SIGTERM, Ctrl-C, or IPC Shutdown command).
/// Returns Ok(()) on clean shutdown, Err on fatal startup failure.
///
/// # Startup sequence
///
/// 1. Resolve XDG paths, ensure directories with correct permissions
/// 2. Harden process (disable core dumps, set non-dumpable, resource limits)
/// 3. Create PID file (detect stale instance, fail if already running)
/// 4. Initialize tracing (structured, runtime-adjustable level)
/// 5. Generate or load IPC bus keypair
/// 6. Bind IPC socket (0600 permissions, stale socket cleanup)
/// 7. Apply sandbox (Landlock + seccomp — AFTER socket bind, BEFORE traffic)
/// 8. Construct DaemonContext (minimal — everything None until unlock)
/// 9. Start event delivery system
/// 10. Register daemon subscriber on own socket
/// 11. Transition to LOCKED (sd_notify READY=1)
/// 12. Start config watcher (inotify on config directory)
/// 13. Start metrics endpoint (Prometheus on localhost)
/// 14. Start health check endpoint
/// 15. Enter main select! loop (accept, signals, shutdown, watchdog)
///
/// # Shutdown sequence (reverse order)
///
/// 1. Transition to ShuttingDown (sd_notify STOPPING=1)
/// 2. Stop accepting new IPC connections
/// 3. Drain in-flight requests (configurable timeout, default 10s)
/// 4. Abort daemon subscriber + event consumer
/// 5. Drop BusServer (removes socket file)
/// 6. Remove PID file
/// 7. Transition to Stopped
#[allow(clippy::too_many_lines)]
pub async fn run_daemon() -> anyhow::Result<()> {
    // ── 0. Initialize tracing with runtime-adjustable level ────────
    tracing_init::init_tracing();

    // ── 1. Resolve paths ────────────────────────────────────────────
    let paths = StatePaths::resolve()?;
    paths.ensure_directories().await?;
    tracing::info!(
        state_dir = %paths.state_dir.display(),
        veilid_dir = %paths.veilid_dir.display(),
        config_dir = %paths.config_dir.display(),
        "state directories ready"
    );

    // ── 2. Harden process ───────────────────────────────────────────
    hardening::harden_process();
    hardening::apply_resource_limits(&hardening::ResourceLimits {
        nofile: 65536,
        memlock_bytes: 64 * 1024 * 1024, // 64 MB for mlock'd secrets
    });

    // ── 3. PID file ─────────────────────────────────────────────────
    let pid_guard = pid::PidFile::acquire(&paths.state_dir.join("rekindle.pid"))?;
    tracing::info!(pid = std::process::id(), "PID file acquired");

    // ── 4. Lifecycle state machine ──────────────────────────────────
    let lifecycle = Arc::new(DaemonLifecycle::new());
    assert!(lifecycle.transition(DaemonState::Starting), "initial Stopped→Starting must succeed");

    // ── 5. IPC bus keypair ──────────────────────────────────────────
    let runtime_dir = ipc::runtime_dir()?;
    tokio::fs::create_dir_all(&runtime_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    ipc::noise_keys::create_keys_dir().await?;

    let bus_keypair = load_or_generate_bus_keypair(&runtime_dir).await?;

    // ── 6. Bind IPC socket ──────────────────────────────────────────
    let socket_path = ipc::socket_path()?;

    let daemon_subscriber_kp = ipc::generate_keypair()
        .map_err(|e| anyhow::anyhow!("daemon subscriber keypair: {e}"))?;
    let daemon_subscriber_pubkey: [u8; 32] = daemon_subscriber_kp
        .as_inner()
        .public
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("subscriber pubkey not 32 bytes"))?;

    let mut clearance_registry = ipc::ClearanceRegistry::new();
    clearance_registry.register(
        ipc::server::DAEMON_AGENT_NAME.to_string(),
        daemon_subscriber_pubkey,
        ipc::SecurityLevel::Internal,
        ipc::AgentType::System,
        vec!["dispatch".into()],
    );

    let encrypt_pool = crate::ipc::bulk::build_encrypt_pool();
    let bulk_buffer_pool = crate::ipc::bulk::BufferPool::new();
    let bulk_counters = crate::ipc::bulk::BulkCounters::new();
    let event_journal = crate::ipc::journal::EventJournal::new();

    let bus_server = ipc::BusServer::bind(
        &socket_path,
        bus_keypair.into_inner(),
        clearance_registry,
        Arc::clone(&encrypt_pool),
        bulk_buffer_pool,
        Arc::clone(&bulk_counters),
        Arc::clone(&event_journal),
    )?;
    tracing::info!(path = %socket_path.display(), "IPC bus server bound");

    // ── 7. Sandbox ──────────────────────────────────────────────────
    // Load config early so sandbox gets the configured metrics/health ports.
    let early_config = crate::daemon::dispatch::lifecycle::load_early_config(&paths);
    sandbox::apply(&paths, &socket_path, &early_config);

    // ── 8. DaemonContext ────────────────────────────────────────────
    let (event_watch_tx, event_watch_rx) = tokio::sync::watch::channel(None);

    let daemon_ctx = Arc::new(DaemonContext {
        chat: parking_lot::RwLock::new(None),
        transport: parking_lot::RwLock::new(None),
        vault: parking_lot::RwLock::new(None),
        lifecycle: Arc::clone(&lifecycle),
        paths: paths.clone(),
        registry: Arc::new(tokio::sync::RwLock::new(ipc::ClearanceRegistry::new())),
        policy: parking_lot::RwLock::new(
            crate::daemon::dispatch::PolicyConfig::default(),
        ),
        event_watch_tx,
        status_cache: parking_lot::Mutex::new(None),
        encrypt_pool,
        bulk_counters,
        bulk_transfers: parking_lot::Mutex::new(
            crate::daemon::dispatch::bulk_transfers::BulkTransferRegistry::new(),
        ),
        event_journal,
        idempotency_cache: crate::ipc::idempotency::IdempotencyCache::new(),
        server_state: parking_lot::RwLock::new(Some(bus_server.state())),
    });

    // ── 9. Event delivery ───────────────────────────────────────────
    bus_server.start_event_delivery(event_watch_rx);

    // ── 10. Transition to LOCKED ────────────────────────────────────
    assert!(lifecycle.transition(DaemonState::Locked), "Starting→Locked must succeed");
    systemd::notify_ready();
    systemd::notify_status("locked — awaiting unlock");
    tracing::info!(state = "locked", "daemon accepting connections");

    // ── 11. Daemon bus subscriber ───────────────────────────────────
    //
    // Two channels:
    // - RoutedFrame channel (server → daemon subscriber): structured request
    //   forwarding with routing header + raw bytes. No decode+re-encode.
    // - BusClient connection (daemon subscriber → server): response path.
    //   The subscriber sends correlated responses back through the bus.
    let (routed_tx, routed_rx) = tokio::sync::mpsc::channel::<ipc::RoutedFrame>(1024);
    bus_server.register_daemon_channel(routed_tx);

    let daemon_ctx_subscriber = Arc::clone(&daemon_ctx);
    let subscriber_socket = socket_path.clone();
    let subscriber_handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;

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
            Duration::from_millis(200),
        )
        .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "daemon subscriber: connect failed");
                return;
            }
        };

        tracing::info!("daemon bus subscriber connected");
        let responder = client.responder();
        daemon_ctx_subscriber.run_subscriber(routed_rx, responder).await;
    });

    // ── 12. Config watcher ──────────────────────────────────────────
    let _config_watcher = config_watch::start_config_watcher(
        &paths.config_dir,
        Arc::clone(&daemon_ctx),
    );

    // ── 13. Metrics endpoint ────────────────────────────────────────
    let metrics_state = Arc::new(metrics::DaemonMetrics::new());
    let metrics_handle = tokio::spawn(metrics::serve_prometheus(
        Arc::clone(&metrics_state),
        early_config.metrics_port,
    ));

    // ── 14. Health check endpoint ───────────────────────────────────
    let health_lifecycle = Arc::clone(&lifecycle);
    let health_handle = tokio::spawn(health::serve_health(health_lifecycle, early_config.health_port));

    // ── 15. Signal handlers ─────────────────────────────────────────
    let signal_ctx = Arc::clone(&daemon_ctx);
    let signal_lifecycle = Arc::clone(&lifecycle);

    // ── 16. Main loop ───────────────────────────────────────────────
    //
    // Wrapped in a loop so non-terminal signals (SIGHUP, SIGUSR1, SIGUSR2)
    // are handled without exiting the select!. Only terminal signals
    // (SIGTERM, SIGINT) and IPC shutdown break the loop.
    //
    // Signal coalescing: tokio signals are coalesced — if SIGHUP fires
    // twice while the watchdog branch is being processed, only one
    // SIGHUP notification is delivered. This is acceptable for config
    // reload (idempotent operation).
    let mut watchdog_interval = tokio::time::interval(Duration::from_secs(15));
    watchdog_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut signal_stream = signals::SignalStream::new()?;

    let shutdown_reason: &str;

    loop {
        tokio::select! {
            result = bus_server.run() => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "bus server fatal error");
                }
                shutdown_reason = "bus server exited";
                break;
            }
            () = lifecycle.shutdown_requested() => {
                shutdown_reason = "IPC shutdown request";
                break;
            }
            signal = signal_stream.next() => {
                match signal {
                    Some(signals::Signal::Terminate) => {
                        tracing::info!("SIGTERM received");
                        shutdown_reason = "SIGTERM";
                        break;
                    }
                    Some(signals::Signal::Interrupt) => {
                        tracing::info!("SIGINT received");
                        shutdown_reason = "SIGINT";
                        break;
                    }
                    Some(signals::Signal::HangUp) => {
                        tracing::info!("SIGHUP received — reloading config");
                        config_watch::reload_config(&signal_ctx);
                        // Continue the loop — SIGHUP is non-terminal.
                    }
                    Some(signals::Signal::User1) => {
                        tracing::info!("SIGUSR1 — writing diagnostic dump");
                        health::write_diagnostic_dump(&signal_ctx, &paths).await;
                        // Continue the loop — SIGUSR1 is non-terminal.
                    }
                    Some(signals::Signal::User2) => {
                        tracing::info!("SIGUSR2 — rotating log level");
                        signals::rotate_log_level();
                        // Continue the loop — SIGUSR2 is non-terminal.
                    }
                    None => {
                        tracing::error!("signal stream closed unexpectedly");
                        shutdown_reason = "signal stream closed";
                        break;
                    }
                }
            }
            _ = watchdog_interval.tick() => {
                systemd::notify_watchdog();
                systemd::notify_status(signal_lifecycle.state().as_str());
                metrics_state.update_from_context(&signal_ctx);
                // Continue the loop — watchdog is periodic.
            }
        }
    }

    tracing::info!(reason = shutdown_reason, "initiating shutdown");

    // ── Shutdown ────────────────────────────────────────────────────
    let _ = lifecycle.transition(DaemonState::ShuttingDown);
    systemd::notify_status("shutting down");
    tracing::info!("draining connections...");

    // Graceful drain: wait up to 10s for in-flight requests.
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    tokio::select! {
        () = tokio::time::sleep_until(drain_deadline) => {
            tracing::warn!("drain timeout reached — force-closing connections");
        }
        () = async {
            loop {
                if bus_server.connection_count() == 0 { break; }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } => {
            tracing::info!("all connections drained cleanly");
        }
    }

    // Abort background tasks.
    subscriber_handle.abort();
    metrics_handle.abort();
    health_handle.abort();
    let _ = subscriber_handle.await;
    let _ = metrics_handle.await;
    let _ = health_handle.await;

    // Drop bus server (removes socket file via Drop impl).
    drop(bus_server);

    // Remove PID file.
    drop(pid_guard);

    let _ = lifecycle.transition(DaemonState::Stopped);
    tracing::info!("rekindle daemon stopped");

    Ok(())
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

/// Load the bus server keypair with BLAKE3 tamper detection.
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

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}
