//! `rekindled`: the composition root of the one backend host (ADR 0010,
//! plan C1).
//!
//! Takes the data root's `NodeLock`, loads policy and the bus key, starts
//! the transport, builds the `DaemonContext` and its workers, binds the IPC
//! bus and serves until shutdown. Moved out of the CLI, which is now a
//! client of this binary like every other frontend.

pub mod bus_key;
pub mod policy;
mod signals;
pub mod watchdog;

use std::sync::Arc;

use parking_lot::RwLock;

use crate::daemon::dispatch::DaemonContext;
use crate::daemon::handler::DaemonHandler;
use crate::daemon::shutdown::{ExitReason, BUS_DRAIN_DEADLINE, WORKER_STOP_DEADLINE};
use crate::daemon::{DaemonLifecycle, DaemonState};
use rekindle_db::lock::NodeLock;
use rekindle_db::paths::DataRoot;

use rekindle_transport::crypto::mek::MekCache;

/// `rekindled`'s command line. The host takes no options yet; parsing still
/// answers `--help` and `--version`.
#[derive(Debug, Clone, clap::Parser)]
#[command(name = "rekindled", version, about = "The Rekindle backend host")]
pub struct HostArgs {}

/// Log to `<log_dir>/rekindled.log` (daily rotation) through the identifier
/// scrubber every Rekindle log line passes. The returned guard flushes the
/// writer on drop and must live as long as the process.
///
/// # Errors
/// No state directory can be resolved, or the log directory cannot be
/// created. The daemon does not run unlogged.
pub fn init_tracing() -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    let log_dir = DataRoot::resolve()?.logs;
    std::fs::create_dir_all(&log_dir)
        .map_err(|e| anyhow::anyhow!("cannot create log directory {}: {e}", log_dir.display()))?;
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(log_dir, "rekindled.log"));
    tracing_subscriber::fmt()
        .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(writer))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rekindle=info,warn")),
        )
        .with_ansi(false)
        .init();
    Ok(guard)
}

/// Run the host until Ctrl-C, an IPC shutdown or a handler panic, then shut
/// down gracefully. Returns why it stopped, which decides the exit status.
///
/// # Errors
/// Startup failures: the data root is locked by another `rekindled`, the
/// policy does not parse, or the bus cannot be bound.
pub async fn run(args: HostArgs) -> anyhow::Result<ExitReason> {
    tracing::debug!(?args, "rekindled starting");
    // ── 1. Resolve paths and ensure directories ───────────────────
    let root = DataRoot::resolve()?;
    root.create_dirs()?;

    // One node per data root, before anything below touches state or the
    // bus socket: the desktop takes the same lock (plan C5). Held for the
    // life of the process.
    let _node_lock = NodeLock::acquire(&root.data)?;
    let session_file = root.state.join("session.json");
    let veilid_dir = root.veilid();
    tracing::info!(
        data = %root.data.display(),
        state = %root.state.display(),
        "data root ready"
    );

    // ── 2. Initialize daemon lifecycle ────────────────────────────
    let lifecycle = Arc::new(DaemonLifecycle::new());
    lifecycle.transition(DaemonState::Starting)?;

    // ── 3. Generate or load bus keypair ───────────────────────────
    let runtime_dir = rekindle_ipc::runtime_dir()?;
    tokio::fs::create_dir_all(&runtime_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    rekindle_ipc::noise_keys::create_keys_dir().await?;

    let bus_keypair = bus_key::load_or_generate(&runtime_dir)
        .await
        .map_err(ConfigFault)?;

    // ── 4. Load session from disk ─────────────────────────────────
    let session = crate::state::load_session(&session_file)?;
    let has_identity = session.is_some();
    tracing::info!(has_identity, "session loaded");

    // ── 5. Build transport config and start Veilid node ───────────
    //
    // Admin policy is applied here, before the node starts, from the same
    // loader `PolicyReload` uses. A policy file that exists but does not
    // parse stops startup: running unconstrained would silently drop a
    // constraint an administrator meant to impose. `min_hop_count` is a
    // floor on the safety-route length, alongside the transport's own
    // `ANONYMITY_HOP_FLOOR`: an administrator can raise the floor, never
    // lower it.
    let policy = policy::load_layered().map_err(|e| ConfigFault(e.into()))?;

    // `[network]` of the layered `config.toml`, the same file and schema
    // `rekindle config validate` checks. A file that does not parse or
    // validate stops startup with EX_CONFIG.
    let user_config: rekindle_types::config::user::UserConfig =
        rekindle_utils::config_layers::load(None).map_err(|e| ConfigFault(e.into()))?;
    user_config.validate().map_err(|e| ConfigFault(e.into()))?;
    let mut transport_config = rekindle_transport::TransportConfig {
        storage_dir: veilid_dir.display().to_string(),
        ..user_config.network
    };
    if let Some(ceiling) = policy.max_gossip_ttl {
        if transport_config.gossip_ttl > ceiling {
            tracing::info!(gossip_ttl = ceiling, "admin policy capped the gossip TTL");
            transport_config.gossip_ttl = ceiling;
        }
    }
    if let Some(floor) = policy.min_hop_count {
        for profile in [
            &mut transport_config.safety.text,
            &mut transport_config.safety.voice,
            &mut transport_config.safety.dht,
            &mut transport_config.safety.rpc,
        ] {
            profile.hop_count = profile.hop_count.max(floor);
        }
        tracing::info!(
            min_hop_count = floor,
            "admin policy raised the safety-route floor"
        );
    }

    let session_arc = Arc::new(parking_lot::RwLock::new(session));
    let mek_cache = Arc::new(parking_lot::RwLock::new(MekCache::new()));
    let signing_key_arc: Arc<
        parking_lot::RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    > = Arc::new(parking_lot::RwLock::new(None));
    // The one clearance registry: dispatch registers agents into it and the
    // bus server authenticates against it.
    let registry = Arc::new(tokio::sync::RwLock::new(
        rekindle_ipc::ClearanceRegistry::new(),
    ));

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
    let (mek_rotation_tx, mek_rotation_rx) = crate::daemon::mek_rotation::channel();

    // Outbound gossip (PATH 2). Every adapter's `send_to_mesh` queues
    // here so one encoder puts one wire format on the network; the
    // worker below owns the `Arc<DaemonContext>` and the single
    // `ResolveGate` that route re-resolution coalesces through.
    let (gossip_tx, gossip_rx) = crate::daemon::gossip::channel();

    // Presence polls are requested on unlock; the supervisor below owns
    // the `Arc<DaemonContext>` the long-lived poll loops need.
    let (presence_start_tx, presence_start_rx) =
        crate::daemon::presence_adapter::supervisor::channel();

    let handler = Arc::new(DaemonHandler::new(
        Arc::clone(&transport_subscriptions),
        Arc::clone(&session_arc),
        session_file.clone(),
        Arc::clone(&mek_cache),
        Arc::clone(&signing_key_arc),
        Arc::clone(&transport_for_handler),
        Arc::clone(&pending_joins),
        mek_rotation_tx.clone(),
        gossip_tx.clone(),
    ));

    // A daemon without its transport cannot do anything it exists for;
    // it exits non-zero and its supervisor retries the start.
    let node = rekindle_transport::TransportNode::start(
        transport_config,
        handler,
        Arc::clone(&session_arc),
    )
    .await
    .map_err(|e| anyhow::anyhow!("transport node did not start: {e}"))?;
    let transport = Arc::new(node);
    // Fill the handler's transport reference now that it's started.
    *transport_for_handler.write() = Some(Arc::clone(&transport));
    tracing::info!("transport node started");

    // ── 6. Create DaemonContext ───────────────────────────────────
    // The subscriber beats twice per watchdog ping, so each ping sees a
    // beat no older than one ping interval.
    let keepalive = watchdog::keepalive_interval();
    // No unaudited operation: a daemon that cannot open its audit log
    // does not start.
    let audit_logger = crate::state::audit::AuditLogger::open(&root.state.join("audit.jsonl"))
        .map_err(|e| anyhow::anyhow!("audit log did not open: {e}"))?;
    tracing::info!(
        sequence = audit_logger.sequence(),
        "audit logger initialized"
    );

    // Watch channel for event source notification.
    // handle_unlock sends the broadcast::Sender through this when SubscriptionManager
    // is created. The IPC server receives it and starts its internal delivery task.
    let (event_watch_tx, event_watch_rx) = tokio::sync::watch::channel(None);

    let daemon_ctx = Arc::new(DaemonContext {
        community_runtime: Arc::new(crate::daemon::community_runtime::CommunityRuntimeMap::new()),
        transport: RwLock::new(Some(transport)),
        session: session_arc,
        mek_cache,
        signing_key: signing_key_arc,
        lifecycle: Arc::clone(&lifecycle),
        session_path: session_file.clone(),
        registry: Arc::clone(&registry),
        // The policy the daemon actually started under, so
        // `PolicyReload` merges against reality rather than defaults.
        policy: RwLock::new(policy),
        config_dir: rekindle_utils::config_layers::user_dir().map_err(|e| ConfigFault(e.into()))?,
        audit: parking_lot::Mutex::new(Some(audit_logger)),
        subscriptions: RwLock::new(None),
        broadcast_mgr: RwLock::new(None),
        write_lane: tokio::sync::RwLock::new(()),
        subscriber_heartbeat: crate::daemon::heartbeat::Heartbeat::new(
            keepalive.map_or(crate::daemon::heartbeat::DEFAULT_TICK, |ping| ping / 2),
        ),
        shutdown: Arc::new(crate::daemon::shutdown::Shutdown::new()),
        event_watch_tx,
        pending_joins: Arc::clone(&pending_joins),
        gossip_tx,
        mek_rotation_tx,
        unlock_scope: parking_lot::RwLock::new(None),
        community_scopes: parking_lot::Mutex::new(std::collections::HashMap::new()),
        presence_start_tx,
        status: Arc::new(RwLock::new(rekindle_presence::UserStatusKind::Online)),
        status_wake: Arc::new(tokio::sync::Notify::new()),
    });

    // The rotation worker owns an `Arc<DaemonContext>` — the reason the
    // triggers queue instead of spawning. It waits out the cascade
    // (tens of seconds) off the request path.
    let workers = tokio_util::task::TaskTracker::new();
    workers.spawn(crate::daemon::mek_rotation::run_worker(
        Arc::clone(&daemon_ctx),
        mek_rotation_rx,
    ));

    // Gossip worker. Holds one adapter for its lifetime so concurrent
    // route re-resolutions actually coalesce through a shared gate.
    workers.spawn(crate::daemon::gossip::run_worker(
        Arc::clone(&daemon_ctx),
        gossip_rx,
    ));

    // Presence supervisor. Spawns one registry-scan loop per community
    // when unlock asks; those loops are what give the daemon a validated
    // member roster at all.
    workers.spawn(crate::daemon::presence_adapter::supervisor::run_supervisor(
        Arc::clone(&daemon_ctx),
        presence_start_rx,
    ));

    // ── 7. Bind IPC socket and create bus server ──────────────────
    let socket_path = rekindle_ipc::socket_path()?;

    // Generate the daemon subscriber's keypair BEFORE binding the server.
    // This keypair is registered in the clearance registry so the server
    // authenticates the subscriber at Internal level when it connects.
    let daemon_subscriber_kp = rekindle_ipc::generate_keypair()
        .map_err(|e| anyhow::anyhow!("daemon subscriber keypair generation failed: {e}"))?;
    let daemon_subscriber_pubkey: [u8; 32] = daemon_subscriber_kp
        .as_inner()
        .public
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("subscriber pubkey is not 32 bytes"))?;

    registry.write().await.register(
        rekindle_ipc::server::DAEMON_AGENT_NAME.to_string(),
        daemon_subscriber_pubkey,
        rekindle_ipc::SecurityLevel::Internal,
        rekindle_ipc::AgentType::System,
        vec!["dispatch".into()],
    );

    let bus_server = rekindle_ipc::BusServer::bind(
        &socket_path,
        bus_keypair.into_inner(),
        Arc::clone(&registry),
    )?;
    tracing::info!(path = %socket_path.display(), "IPC bus server bound");

    // Start the event delivery system. The delivery task awaits the watch
    // channel for the broadcast sender (populated during handle_unlock).
    // Events route in-process through the EventRouter — no bridge task needed.
    bus_server.start_event_delivery(event_watch_rx);

    // ── 8. Transition to Locked ───────────────────────────────────
    lifecycle.transition(DaemonState::Locked)?;
    tracing::info!(
        state = lifecycle.state().as_str(),
        "daemon accepting connections"
    );

    // ── 9. Spawn the daemon's bus subscriber ─────────────────────
    // The daemon connects to its own socket as a privileged internal
    // agent using the keypair registered in the clearance registry.
    // Requests are unicast to this connection by the server. The
    // subscriber sends READY=1 once connected: before that the bus
    // answers every request 503, so "ready" means serving.
    let mut subscriber_handle = spawn_bus_subscriber(
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
    workers.spawn(run_event_consumer(Arc::clone(&daemon_ctx)));

    // ── 10. Run IPC accept loop with shutdown signal + watchdog ───
    let mut stop_signals = signals::StopSignals::register()?;
    tokio::select! {
        result = bus_server.run() => {
            if let Err(e) = result {
                tracing::error!(error = %e, "bus server fatal error");
            }
        }
        signal = stop_signals.next() => {
            tracing::info!(signal, "shutdown signal received");
        }
        () = daemon_ctx.shutdown.requested() => {
            tracing::info!(reason = ?daemon_ctx.shutdown.reason(), "shutdown requested");
        }
        () = watchdog::keep_alive(keepalive, &daemon_ctx.subscriber_heartbeat) => {}
    }

    // ── 11. Graceful shutdown ─────────────────────────────────────
    watchdog::notify_stopping();
    tracing::info!("draining connections...");

    // The subscriber stops reading, answers what is in flight (bounded by
    // `REQUEST_DRAIN_DEADLINE`) and flushes its connection; only then is
    // the bus dropped, so a `node stop` gets its reply.
    daemon_ctx.shutdown.request(ExitReason::Requested);
    if let Err(e) = (&mut subscriber_handle).await {
        tracing::error!(error = %e, "bus subscriber task failed");
    }
    // After the drain, so no request is mid-transition. Shutdown finishes
    // either way: a refused edge (a request aborted at the deadline while
    // Resuming or Locking) is logged, and the teardown below runs anyway.
    settle(&lifecycle, DaemonState::ShuttingDown);
    bus_server.shutdown(BUS_DRAIN_DEADLINE).await;
    drop(bus_server);
    // The unlock's tasks first: closing its scope stops the presence
    // polls, the subscription loops and any MEK rotation at their next
    // safe point, so no Veilid call is cut mid-flight (plan C4.L1).
    crate::daemon::dispatch::teardown_unlocked(&daemon_ctx).await;
    // The workers saw the same shutdown signal and stop at their next
    // await (each documents why cutting its current item is safe).
    workers.close();
    if tokio::time::timeout(WORKER_STOP_DEADLINE, workers.wait())
        .await
        .is_err()
    {
        tracing::warn!(
            running = workers.len(),
            "workers still running after the stop deadline"
        );
    }

    shutdown_transport(&daemon_ctx).await;

    let reason = daemon_ctx.shutdown.reason();
    if reason == ExitReason::DataWiped {
        // Only now: the transport held this storage open until it stopped.
        match std::fs::remove_dir_all(&veilid_dir) {
            Ok(()) => tracing::info!("veilid storage wiped"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => anyhow::bail!(
                "wipe incomplete: cannot delete {}: {e}",
                veilid_dir.display()
            ),
        }
    }

    settle(&lifecycle, DaemonState::Stopped);
    tracing::info!(?reason, "rekindle daemon stopped");

    Ok(reason)
}

/// A startup failure that a restart cannot fix: the policy does not parse,
/// or the bus keypair is incomplete, malformed or tampered with. `rekindled`
/// exits `EX_CONFIG` (78), which the unit lists in
/// `RestartPreventExitStatus=` so systemd does not loop on it.
#[derive(Debug)]
pub struct ConfigFault(pub anyhow::Error);

impl std::fmt::Display for ConfigFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for ConfigFault {}

impl ConfigFault {
    /// sysexits(3) EX_CONFIG: "Something was found in an unconfigured or
    /// misconfigured state."
    pub const EXIT_CODE: u8 = 78;
}

/// Move the lifecycle during shutdown, logging an edge the FSM refuses
/// instead of abandoning the shutdown.
fn settle(lifecycle: &DaemonLifecycle, next: DaemonState) {
    if let Err(e) = lifecycle.transition(next) {
        tracing::error!(error = %e, "lifecycle refused a shutdown transition");
    }
}

/// Spawn the daemon's own bus subscriber.
///
/// The daemon connects to its own socket as a privileged internal agent
/// using the keypair registered in the clearance registry. Requests are
/// unicast to this connection by the server.
fn spawn_bus_subscriber(
    daemon_ctx: Arc<DaemonContext>,
    subscriber_socket: std::path::PathBuf,
    daemon_subscriber_kp: rekindle_ipc::ZeroizingKeypair,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let server_pub = match rekindle_ipc::noise_keys::read_bus_public_key().await {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(error = %e, "daemon subscriber: cannot read bus public key");
                daemon_ctx.shutdown.request(ExitReason::SubscriberFailed);
                return;
            }
        };

        let sender_id = uuid::Uuid::now_v7();
        let client = match rekindle_ipc::BusClient::connect_with_retry(
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
                daemon_ctx.shutdown.request(ExitReason::SubscriberFailed);
                return;
            }
        };

        tracing::info!("daemon bus subscriber connected");
        watchdog::notify_ready();
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

/// The daemon-internal tier-3 event consumer: each `ValueChanged` that
/// tier 1 missed triggers the matching inbox scan. It follows the event
/// source across lock cycles (`rekindle_ipc::event_source::follow`), so a Lock →
/// Unlock cycle never leaves the daemon without its tier-3 inbox path.
async fn run_event_consumer(daemon_ctx: Arc<DaemonContext>) {
    let source = daemon_ctx.event_watch_tx.subscribe();
    rekindle_ipc::event_source::follow(daemon_ctx.shutdown.token(), source, |event| {
        let daemon_ctx = Arc::clone(&daemon_ctx);
        async move {
            if let rekindle_types::subscription_events::SubscriptionEvent::Network(
                rekindle_types::subscription_events::NetworkEvent::ValueChanged {
                    record_key, ..
                },
            ) = event
            {
                handle_value_changed(&daemon_ctx, &record_key).await;
            }
        }
    })
    .await;
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
        crate::daemon::friend_inbox::scan_friend_inbox(
            &daemon_ctx.session,
            &daemon_ctx.transport,
            &daemon_ctx.session_path,
            &inbox_key,
        )
        .await;
    }
}
