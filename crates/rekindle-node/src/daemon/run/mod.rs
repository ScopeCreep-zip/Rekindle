//! Daemon process runtime — orchestrates all OS-level concerns.
//!
//! `run_daemon()` is the process entry point. It owns process hardening,
//! PID file lifecycle, IPC server (via transport-ipc), signal handling,
//! systemd integration, sandbox, config hot-reload, health check, and
//! graceful shutdown.

pub mod signals;
pub mod pid;
pub mod sandbox;
pub mod hardening;
pub mod systemd;
pub mod config_watch;
pub mod metrics;
#[allow(unsafe_code)]
pub mod health;
pub mod tracing_init;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_transport_ipc::v3::bulk::counters::BulkCounters;
use rekindle_transport_ipc::v3::context::ServerConfig;
use rekindle_transport_ipc::v3::server::IpcServer;

use crate::daemon::{DaemonLifecycle, DaemonState};
use crate::daemon::dispatch::DaemonContext;
use crate::daemon::dispatch::bulk_transfers::BulkTransferRegistry;
use crate::idempotency::IdempotencyCache;
use crate::journal::EventJournal;
use crate::state::StatePaths;
use crate::subscriptions::SubscriptionRegistry;

/// Run the rekindle daemon. This is the sole process entry point.
#[allow(clippy::too_many_lines)]
pub async fn run_daemon() -> anyhow::Result<()> {
    // ── 0. Initialize tracing ───────────────────────────────────────
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
        memlock_bytes: 64 * 1024 * 1024,
    });

    // ── 3. PID file ─────────────────────────────────────────────────
    let pid_guard = pid::PidFile::acquire(&paths.state_dir.join("rekindle.pid"))?;
    tracing::info!(pid = std::process::id(), "PID file acquired");

    // ── 4. Lifecycle state machine ──────────────────────────────────
    let lifecycle = Arc::new(DaemonLifecycle::new());
    assert!(lifecycle.transition(DaemonState::Starting), "initial Stopped→Starting must succeed");

    // ── 5. Application-level keypair ────────────────────────────────
    let runtime_dir = rekindle_keys::runtime_dir()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    tokio::fs::create_dir_all(&runtime_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    rekindle_keys::create_keys_dir().await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let keypair = load_or_generate_keypair(&runtime_dir).await?;

    // ── 6. Socket path ──────────────────────────────────────────────
    let socket_path = rekindle_keys::socket_path()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // ── 7. Shared counters + application state ──────────────────────
    //
    // BulkCounters is constructed once, shared by DaemonContext (reads
    // for status/metrics/health) and ServerConfig (transport writes).
    // One Arc, two holders, no Option, no temporal gap.
    let transport_counters = BulkCounters::new();
    let subscriptions = SubscriptionRegistry::new();

    let daemon_ctx = Arc::new(DaemonContext {
        chat: parking_lot::RwLock::new(None),
        transport: parking_lot::RwLock::new(None),
        vault: parking_lot::RwLock::new(None),
        lifecycle: Arc::clone(&lifecycle),
        paths: paths.clone(),
        policy: parking_lot::RwLock::new(
            crate::daemon::dispatch::PolicyConfig::default(),
        ),
        status_cache: parking_lot::Mutex::new(None),
        idempotency_cache: IdempotencyCache::new(),
        event_journal: EventJournal::new(),
        subscriptions: Arc::clone(&subscriptions),
        transport_counters: Arc::clone(&transport_counters),
        agents: RwLock::new(HashMap::new()),
        bulk_transfers: Mutex::new(BulkTransferRegistry::new()),
    });

    // ── 8. Bind IpcServer ───────────────────────────────────────────
    let ctx_for_factory = Arc::clone(&daemon_ctx);
    let subs_for_factory = Arc::clone(&subscriptions);

    let server = IpcServer::bind(
        &socket_path,
        keypair.into_inner(),
        move |conn_handle| {
            crate::routing::DaemonRouter::new(
                conn_handle,
                Arc::clone(&ctx_for_factory),
                Arc::clone(&subs_for_factory),
            )
        },
        ServerConfig {
            counters: Arc::clone(&transport_counters),
            ..ServerConfig::new()
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("IpcServer::bind failed: {e:?}"))?;

    tracing::info!(path = %socket_path.display(), "IPC server bound");

    // ── 9. Sandbox (after socket bind, before traffic) ──────────────
    let early_config = crate::daemon::dispatch::lifecycle::load_early_config(&paths);
    sandbox::apply(&paths, &socket_path, &early_config);

    // ── 10. Locked state ────────────────────────────────────────────
    assert!(lifecycle.transition(DaemonState::Locked), "Starting→Locked must succeed");
    systemd::notify_ready();
    systemd::notify_status("locked — awaiting unlock");
    tracing::info!(state = "locked", "daemon accepting connections");

    // ── 11. Config watcher ──────────────────────────────────────────
    let _config_watcher = config_watch::start_config_watcher(
        &paths.config_dir,
        Arc::clone(&daemon_ctx),
    );

    // ── 12. Health check endpoint ───────────────────────────────────
    let health_lifecycle = Arc::clone(&lifecycle);
    let health_handle = tokio::spawn(health::serve_health(health_lifecycle, early_config.health_port));

    // ── 13. Main loop ───────────────────────────────────────────────
    let signal_ctx = Arc::clone(&daemon_ctx);
    let mut signal_stream = signals::SignalStream::new()?;

    let shutdown_reason: &str;

    loop {
        tokio::select! {
            result = server.run() => {
                if let Err(e) = result {
                    tracing::error!(error = ?e, "server fatal error");
                }
                shutdown_reason = "server exited";
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
                    }
                    Some(signals::Signal::User1) => {
                        tracing::info!("SIGUSR1 — writing diagnostic dump");
                        health::write_diagnostic_dump(&signal_ctx, &paths).await;
                    }
                    Some(signals::Signal::User2) => {
                        tracing::info!("SIGUSR2 — rotating log level");
                        signals::rotate_log_level();
                    }
                    None => {
                        tracing::error!("signal stream closed unexpectedly");
                        shutdown_reason = "signal stream closed";
                        break;
                    }
                }
            }
        }
    }

    tracing::info!(reason = shutdown_reason, "initiating shutdown");

    // ── Shutdown ────────────────────────────────────────────────────
    let _ = lifecycle.transition(DaemonState::ShuttingDown);
    systemd::notify_status("shutting down");

    health_handle.abort();
    let _ = health_handle.await;

    // Drop server — cancels connections, removes socket file.
    drop(server);

    // Remove PID file.
    drop(pid_guard);

    let _ = lifecycle.transition(DaemonState::Stopped);
    tracing::info!("rekindle daemon stopped");

    Ok(())
}

/// Load the bus server keypair from disk, or generate a fresh one.
async fn load_or_generate_keypair(
    runtime_dir: &std::path::Path,
) -> anyhow::Result<rekindle_keys::ZeroizingKeypair> {
    let pub_path = runtime_dir.join("bus.pub");
    let key_path = runtime_dir.join("bus.key");

    if pub_path.exists() && key_path.exists() {
        match rekindle_keys::load_bus_keypair(&pub_path, &key_path).await {
            Ok(kp) => {
                tracing::info!("loaded existing bus keypair");
                return Ok(kp);
            }
            Err(e) => {
                tracing::warn!(error = %e, "bus keypair load failed, generating fresh");
            }
        }
    }

    let kp = rekindle_keys::generate_keypair()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    rekindle_keys::write_bus_keypair(kp.as_inner()).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    tracing::info!("bus keypair generated");
    Ok(kp)
}
