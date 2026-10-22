//! The one backend host (plan D3.1): `rekindled` and the GUI both run it.
//!
//! [`claim`] takes the data root's `NodeLock` and runs the layout check;
//! [`Host::start`] builds the `DaemonContext` over the identity stores the
//! caller shares with it, takes the transport ([`TransportSource`]) and, when
//! asked, serves the IPC bus. Both run the one unlock pipeline
//! (`host::unlock`). `rekindled` starts its own node and runs every domain
//! task ([`HostMode::Daemon`]); the GUI hands over its outbound-only node and
//! runs the stages not yet ported through its `ShellHooks`, until each
//! Phase E port moves its stage into the host ([`HostMode::Shell`]).

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_db::lock::NodeLock;
use rekindle_db::paths::DataRoot;
use rekindle_db::DbHandle;
use rekindle_idempotency::attempts::AttemptLimiter;
use rekindle_transport::crypto::mek::MekCache;
use rekindle_transport::{Session, TransportConfig, TransportNode};
use rekindle_vault::VaultHandle;

use super::{policy, signals, watchdog, ConfigFault};
use crate::daemon::dispatch::DaemonContext;
use crate::daemon::handler::DaemonHandler;
use crate::daemon::shutdown::{ExitReason, BUS_DRAIN_DEADLINE, WORKER_STOP_DEADLINE};
use crate::daemon::{DaemonLifecycle, DaemonState};

/// A data root this process holds: its `NodeLock` is taken and its layout
/// checked. Only [`claim`] makes one, so the lock is taken once per process
/// (a second `flock` on a new descriptor fails, `rekindle_db::lock`).
pub struct ClaimedRoot {
    root: DataRoot,
    _lock: NodeLock,
}

impl ClaimedRoot {
    /// The claimed data root.
    #[must_use]
    pub fn root(&self) -> &DataRoot {
        &self.root
    }
}

/// Take `root`'s node lock and run the layout check under it, before any
/// store opens (plan D1): a root laid out for another version is reset.
///
/// # Errors
/// Another process holds the root, or the layout check failed.
pub fn claim(root: DataRoot) -> anyhow::Result<ClaimedRoot> {
    root.create_dirs()?;
    let lock = NodeLock::acquire(&root.data)?;
    rekindle_db::reset::apply(&root)?;
    tracing::info!(
        data = %root.data.display(),
        state = %root.state.display(),
        "data root claimed"
    );
    Ok(ClaimedRoot { root, _lock: lock })
}

/// The unlocked identity's slots, one set per process: the GUI's `AppState`
/// holds these same `Arc`s, so a bus client and the window see one identity.
#[derive(Clone)]
pub struct HostStores {
    pub db: Arc<DbHandle>,
    pub vault: VaultHandle,
    /// The one wrong-passphrase counter per identity (U1.9).
    pub attempts: Arc<AttemptLimiter<String>>,
    /// The unlocked identity's tamper-evident audit chain.
    pub audit_chain: Arc<super::audit::ChainSlot>,
    /// The unlocked identity's session projection (`host::session`).
    pub session: Arc<RwLock<Option<Session>>>,
    /// The one lifecycle FSM (plan E1.1): the Host makes every identity
    /// transition on it, and the GUI's login button reads it.
    pub lifecycle: Arc<DaemonLifecycle>,
}

impl Default for HostStores {
    fn default() -> Self {
        Self {
            db: Arc::new(DbHandle::default()),
            vault: rekindle_vault::new_handle(),
            attempts: Arc::new(AttemptLimiter::default()),
            audit_chain: Arc::new(super::audit::ChainSlot::new(None)),
            session: Arc::new(RwLock::new(None)),
            lifecycle: Arc::new(DaemonLifecycle::new()),
        }
    }
}

/// Where the host's transport comes from.
pub enum TransportSource {
    /// `rekindled`: start a node with this configuration (admin policy is
    /// applied to it in [`Host::start`]).
    Start(TransportConfig),
    /// A node on ports the caller built and runs (the network simulator):
    /// the host runs every domain task over them, as `Start` does (admin
    /// policy is applied to `config` too).
    Ports {
        config: TransportConfig,
        ports: rekindle_records::port::PortSet,
        events: tokio::sync::mpsc::Receiver<rekindle_records::port::PortEvent>,
    },
    /// The GUI's outbound-only node (`TransportNode::adopt_outbound`). The GUI
    /// keeps the Veilid update stream and its receive paths until E5.7.
    Adopted(Arc<TransportNode>),
    /// No node: the E2E server's host, which drives the identity lifecycle
    /// over a temporary root. A request that needs the network answers 503.
    Offline,
}

/// How the host runs (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMode {
    /// `rekindled`: the host started the node and runs every domain task.
    Daemon,
    /// The GUI: the host runs the identity lifecycle over the GUI's adopted
    /// node and calls its `ShellHooks` for the stages the GUI still runs.
    Shell,
}

/// What [`Host::start`] needs.
pub struct HostConfig {
    pub root: ClaimedRoot,
    pub stores: HostStores,
    pub transport: TransportSource,
    /// Bind the IPC bus, so the TUI and CLI attach to this process.
    pub serve_bus: bool,
    /// The wall clock the host and its engines read (plan C9.2):
    /// `SystemClock`, or the network simulator's.
    pub clock: Arc<dyn rekindle_utils::Clock>,
    /// The generator the host and its engines mint ids, keys and jitter
    /// from (plan C9.2): `OsRandom`, or the simulator's seeded one.
    pub rng: Arc<dyn rekindle_utils::RandomSource>,
    /// The GUI's side of an unlock (`ShellHooks`); `None` for `rekindled`,
    /// the E2E server and the simulator.
    pub shell: Option<Arc<dyn super::shell_hooks::ShellHooks>>,
}

/// The bus this host serves, and its own subscriber connection.
pub(super) struct Bus {
    pub(super) server: Arc<rekindle_ipc::BusServer>,
    pub(super) accept: tokio::task::JoinHandle<()>,
    pub(super) subscriber: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// The running host.
pub struct Host {
    pub(crate) ctx: Arc<DaemonContext>,
    mode: HostMode,
    workers: tokio_util::task::TaskTracker,
    bus: Option<Bus>,
    veilid_dir: std::path::PathBuf,
    /// Held for the life of the host.
    _root: ClaimedRoot,
}

impl Host {
    /// Start the host: lifecycle, policy, transport, context, workers and,
    /// with `serve_bus`, the IPC bus. Returns once it accepts requests.
    ///
    /// # Errors
    /// The policy or user config does not parse (`ConfigFault`), the bus key
    /// or socket cannot be set up, or the transport does not start.
    pub async fn start(config: HostConfig) -> anyhow::Result<Arc<Self>> {
        let HostConfig {
            root: claimed,
            stores,
            transport,
            serve_bus,
            clock,
            rng,
            shell,
        } = config;
        let root = claimed.root().clone();
        let veilid_dir = root.create_veilid_dir()?;

        // The shared FSM: the GUI may have moved it to Starting already.
        let lifecycle = Arc::clone(&stores.lifecycle);
        if lifecycle.state() == DaemonState::Stopped {
            lifecycle.transition(DaemonState::Starting)?;
        }

        // Admin policy, from the same loader `PolicyReload` uses. A policy
        // file that exists but does not parse stops startup: running
        // unconstrained would drop a constraint an administrator meant.
        // The user layer is the claimed root's: a host on another root
        // (a test, the network simulator) never reads this user's config.
        let policy = policy::load_layered(&root.config).map_err(|e| ConfigFault(e.into()))?;

        let mek_cache = Arc::new(RwLock::new(MekCache::new()));
        let signing_key = Arc::new(RwLock::new(None));
        let registry = Arc::new(tokio::sync::RwLock::new(
            rekindle_ipc::ClearanceRegistry::new(),
        ));
        let pending_joins = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let (mek_rotation_tx, mek_rotation_rx) = crate::daemon::mek_rotation::channel();
        let (gossip_tx, gossip_rx) = crate::daemon::gossip::channel();
        let (presence_start_tx, presence_start_rx) =
            crate::daemon::presence_adapter::supervisor::channel();

        let friend_host: Arc<std::sync::OnceLock<std::sync::Weak<DaemonContext>>> =
            Arc::new(std::sync::OnceLock::new());
        let daemon_handler = || {
            let handler_transport = Arc::new(RwLock::new(None));
            let handler = Arc::new(DaemonHandler::new(
                Arc::new(RwLock::new(None)),
                Arc::clone(&stores.session),
                Arc::clone(&mek_cache),
                Arc::clone(&signing_key),
                Arc::clone(&handler_transport),
                Arc::clone(&pending_joins),
                mek_rotation_tx.clone(),
                gossip_tx.clone(),
                Arc::clone(&clock),
                Arc::clone(&friend_host),
            ));
            (handler, handler_transport)
        };
        let (mode, node): (HostMode, Option<Arc<TransportNode>>) = match transport {
            TransportSource::Start(transport_config) => {
                let transport_config = apply_policy(
                    TransportConfig {
                        storage_dir: veilid_dir.display().to_string(),
                        ..transport_config
                    },
                    &policy,
                );
                let (handler, handler_transport) = daemon_handler();
                // A daemon without its transport cannot do anything it
                // exists for; it exits non-zero and its supervisor retries.
                let node =
                    TransportNode::start(transport_config, handler, Arc::clone(&stores.session))
                        .await
                        .map_err(|e| anyhow::anyhow!("transport node did not start: {e}"))?;
                let node = Arc::new(node);
                *handler_transport.write() = Some(Arc::clone(&node));
                tracing::info!("transport node started");
                (HostMode::Daemon, Some(node))
            }
            TransportSource::Ports {
                config,
                ports,
                events,
            } => {
                let (handler, handler_transport) = daemon_handler();
                let node = Arc::new(TransportNode::from_ports(
                    apply_policy(config, &policy),
                    ports,
                    events,
                    handler,
                    Arc::clone(&stores.session),
                ));
                *handler_transport.write() = Some(Arc::clone(&node));
                tracing::info!("transport node on supplied ports");
                (HostMode::Daemon, Some(node))
            }
            TransportSource::Adopted(node) => (HostMode::Shell, Some(node)),
            TransportSource::Offline => (HostMode::Shell, None),
        };

        let keepalive = watchdog::keepalive_interval();
        let (event_watch_tx, _) = tokio::sync::watch::channel(None);
        let (unlock_panics, unlock_panics_rx) = tokio::sync::mpsc::unbounded_channel();
        let journal = Arc::new(rekindle_events::SessionJournal::default());
        let ctx = Arc::new_cyclic(|this| DaemonContext {
            this: this.clone(),
            friend_store: RwLock::new(None),
            friend_stores: rekindle_transport::operations::friend::FriendStores::default(),
            outbox: crate::host::adapters::outbox::OutboxSlot::default(),
            friend_watches: super::adapters::friends::FriendWatches::default(),
            own_profile_lease: Arc::new(parking_lot::Mutex::new(None)),
            own_prekey_lease: Arc::new(parking_lot::Mutex::new(None)),
            own_prekey_writes: Arc::new(tokio::sync::Mutex::new(())),
            mode,
            community_runtime: Arc::new(
                crate::daemon::community_runtime::CommunityRuntimeMap::new(),
            ),
            transport: RwLock::new(node.clone()),
            session: Arc::clone(&stores.session),
            root: root.clone(),
            db: Arc::clone(&stores.db),
            vault: stores.vault.clone(),
            identity_attempts: Arc::clone(&stores.attempts),
            mek_cache,
            signing_key,
            lifecycle: Arc::clone(&lifecycle),
            registry: Arc::clone(&registry),
            unlocked_clients: parking_lot::Mutex::new(Vec::new()),
            // The policy the host started under, so `PolicyReload` merges
            // against reality rather than defaults.
            policy: RwLock::new(policy),
            config_dir: root.config.clone(),
            audit_chain: Arc::clone(&stores.audit_chain),
            subscriptions: RwLock::new(None),
            broadcast_mgr: RwLock::new(None),
            write_lane: tokio::sync::RwLock::new(()),
            subscriber_heartbeat: crate::daemon::heartbeat::Heartbeat::new(
                keepalive.map_or(crate::daemon::heartbeat::DEFAULT_TICK, |ping| ping / 2),
            ),
            shutdown: Arc::new(crate::daemon::shutdown::Shutdown::new()),
            journal: Arc::clone(&journal),
            clock: Arc::clone(&clock),
            rng,
            intents: Arc::new(super::gate::HostGate::new(Arc::clone(&stores.attempts))),
            undo: super::undo::UndoCache::default(),
            event_watch_tx,
            pending_joins,
            gossip_tx,
            mek_rotation_tx,
            unlock_scope: RwLock::new(None),
            community_scopes: parking_lot::Mutex::new(std::collections::HashMap::new()),
            presence_start_tx,
            own_presence: super::own_presence::OwnPresence::new(
                Arc::clone(&stores.session),
                node.clone(),
                Arc::clone(&journal),
                Arc::clone(&clock),
            ),
            shell,
            unlock_panics,
        });
        // The inbound handler reaches the friend operations through the
        // context it was built before (plan E2.1).
        if friend_host.set(Arc::downgrade(&ctx)).is_err() {
            tracing::error!("the inbound handler's host was already set");
        }

        // The domain workers each own an `Arc<DaemonContext>`. In the shell
        // the GUI's pipeline runs these stages (one owner per stage), so
        // their queues have no reader there.
        let workers = tokio_util::task::TaskTracker::new();
        if mode == HostMode::Daemon {
            workers.spawn(crate::daemon::mek_rotation::run_worker(
                Arc::clone(&ctx),
                mek_rotation_rx,
            ));
            workers.spawn(crate::daemon::gossip::run_worker(
                Arc::clone(&ctx),
                gossip_rx,
            ));
            workers.spawn(crate::daemon::presence_adapter::supervisor::run_supervisor(
                Arc::clone(&ctx),
                presence_start_rx,
            ));
            workers.spawn(crate::daemon::event_forward::run_forwarder(Arc::clone(
                &ctx,
            )));
        } else {
            // A panicked unlock task ends the session (`host::unlock`).
            workers.spawn(super::unlock::run_panic_teardowns(
                Arc::clone(&ctx),
                unlock_panics_rx,
            ));
        }

        let bus = if serve_bus {
            Some(super::serve::serve(&ctx).await?)
        } else {
            None
        };

        lifecycle.transition(DaemonState::Locked)?;
        tracing::info!(?mode, state = lifecycle.state().as_str(), "host started");

        Ok(Arc::new(Self {
            ctx,
            mode,
            workers,
            bus,
            veilid_dir,
            _root: claimed,
        }))
    }

    /// How this host runs.
    #[must_use]
    pub fn mode(&self) -> HostMode {
        self.mode
    }

    /// Journal `event` for every subscriber (plan D3.2): the GUI's emitters
    /// publish here until their E port moves the emitter into the host.
    pub fn publish(
        &self,
        event: rekindle_types::subscription_events::SubscriptionEvent,
    ) -> rekindle_types::subscription_events::Cursor {
        self.ctx.journal.publish(event)
    }

    /// What followed `from` in the journal, then `Resumed`, or
    /// `InvalidSession` (plan D3.2): a reloading GUI window's replay.
    #[must_use]
    pub fn replay(
        &self,
        from: rekindle_types::subscription_events::Cursor,
    ) -> Vec<rekindle_types::subscription_events::StreamFrame> {
        self.ctx.journal.replay(from)
    }

    /// The data root this host holds.
    #[must_use]
    pub fn root(&self) -> &DataRoot {
        &self.ctx.root
    }

    /// The unlock's scope while an identity is unlocked: the shell's
    /// unported stages run in it (`ShellHooks::unlocked`), and its other
    /// session work spawns there too, so a lock stops all of it.
    #[must_use]
    pub fn unlock_scope(&self) -> Option<Arc<rekindle_lifecycle::SessionScope>> {
        self.ctx.unlock_scope.read().clone()
    }

    /// The host's identity slots, for `host::identity`.
    #[must_use]
    pub fn identity_stores(&self) -> super::identity::Stores<'_> {
        self.ctx.identity_stores()
    }

    /// Wait until the daemon is asked to stop: a stop signal, an IPC
    /// `Shutdown`, a fatal fault, or a missed watchdog beat.
    ///
    /// # Errors
    /// The stop signals cannot be registered.
    pub async fn run_until_stopped(&self) -> anyhow::Result<()> {
        let mut stop_signals = signals::StopSignals::register()?;
        let keepalive = watchdog::keepalive_interval();
        tokio::select! {
            signal = stop_signals.next() => {
                tracing::info!(signal, "shutdown signal received");
            }
            () = self.ctx.shutdown.requested() => {
                tracing::info!(reason = ?self.ctx.shutdown.reason(), "shutdown requested");
            }
            () = watchdog::keep_alive(keepalive, &self.ctx.subscriber_heartbeat) => {}
        }
        Ok(())
    }

    /// Stop the host: drain the bus (every request in flight is answered),
    /// end the unlock, stop the workers and, in the daemon, the transport.
    /// Returns why it stopped, which decides `rekindled`'s exit status.
    ///
    /// # Errors
    /// A wipe could not delete the Veilid storage.
    pub async fn shutdown(&self) -> anyhow::Result<ExitReason> {
        watchdog::notify_stopping();
        tracing::info!("host stopping");
        self.ctx.shutdown.request(ExitReason::Requested);
        if let Some(bus) = &self.bus {
            // The subscriber stops reading, answers what is in flight and
            // flushes its connection; only then is the bus drained, so a
            // `node stop` gets its reply.
            if let Some(subscriber) = bus.subscriber.lock().await.take() {
                if let Err(e) = subscriber.await {
                    tracing::error!(error = %e, "bus subscriber task failed");
                }
            }
            settle(&self.ctx.lifecycle, DaemonState::ShuttingDown);
            bus.server.shutdown(BUS_DRAIN_DEADLINE).await;
            bus.accept.abort();
        } else {
            settle(&self.ctx.lifecycle, DaemonState::ShuttingDown);
        }
        if self.ctx.session.read().is_some() || self.ctx.unlock_scope.read().is_some() {
            super::unlock::teardown(&self.ctx, super::shell_hooks::EndReason::Exit).await;
        }
        self.workers.close();
        if tokio::time::timeout(WORKER_STOP_DEADLINE, self.workers.wait())
            .await
            .is_err()
        {
            tracing::warn!(
                running = self.workers.len(),
                "workers still running after the stop deadline"
            );
        }
        // The shell's node is the GUI's; it stops it itself.
        if self.mode == HostMode::Daemon {
            shutdown_transport(&self.ctx).await;
        }

        let reason = self.ctx.shutdown.reason();
        if reason == ExitReason::DataWiped {
            // Only now: the transport held this storage open until it stopped.
            match std::fs::remove_dir_all(&self.veilid_dir) {
                Ok(()) => tracing::info!("veilid storage wiped"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => anyhow::bail!(
                    "wipe incomplete: cannot delete {}: {e}",
                    self.veilid_dir.display()
                ),
            }
        }
        settle(&self.ctx.lifecycle, DaemonState::Stopped);
        tracing::info!(?reason, "host stopped");
        Ok(reason)
    }
}

/// Apply the admin policy's ceilings and floors to the transport config.
/// `min_hop_count` is a floor on the safety-route length, beside the
/// transport's own `ANONYMITY_HOP_FLOOR`: an administrator can raise it,
/// never lower it.
fn apply_policy(mut config: TransportConfig, policy: &policy::PolicyConfig) -> TransportConfig {
    if let Some(ceiling) = policy.max_gossip_ttl {
        if config.gossip_ttl > ceiling {
            tracing::info!(gossip_ttl = ceiling, "admin policy capped the gossip TTL");
            config.gossip_ttl = ceiling;
        }
    }
    if let Some(floor) = policy.min_hop_count {
        for profile in [
            &mut config.safety.text,
            &mut config.safety.voice,
            &mut config.safety.dht,
            &mut config.safety.rpc,
        ] {
            profile.hop_count = profile.hop_count.max(floor);
        }
        tracing::info!(
            min_hop_count = floor,
            "admin policy raised the safety-route floor"
        );
    }
    config
}

/// The `[network]` section of the layered `config.toml`, the file and schema
/// `rekindle config validate` checks: `rekindled`'s transport config.
///
/// # Errors
/// The file does not parse or validate (`ConfigFault`, `EX_CONFIG`).
pub fn user_transport_config() -> anyhow::Result<TransportConfig> {
    let user_config: rekindle_types::config::user::UserConfig =
        rekindle_utils::config_layers::load(None).map_err(|e| ConfigFault(e.into()))?;
    user_config.validate().map_err(|e| ConfigFault(e.into()))?;
    Ok(user_config.network)
}

/// Move the lifecycle during shutdown, logging an edge the FSM refuses
/// instead of abandoning the shutdown.
fn settle(lifecycle: &DaemonLifecycle, next: DaemonState) {
    if let Err(e) = lifecycle.transition(next) {
        tracing::error!(error = %e, "lifecycle refused a shutdown transition");
    }
}

/// Gracefully shut down the transport node, if one is running.
async fn shutdown_transport(ctx: &Arc<DaemonContext>) {
    let transport = ctx.transport.write().take();
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

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
