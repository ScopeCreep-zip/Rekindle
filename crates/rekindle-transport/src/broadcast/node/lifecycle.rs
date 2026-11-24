//! Transport node startup, adoption, and shutdown.

use std::sync::Arc;

use rekindle_records::port::{PortEvent, PortSet};
use tokio::sync::mpsc;
use tracing::{info, warn, Instrument as _};

use super::routes::{notify_handler, run_route_publisher, OnRoutesChanged};
use super::TransportNode;
use crate::broadcast::peer_registry::PeerRegistry;
use crate::config::TransportConfig;
use crate::error::{Result, TransportError};
use crate::handler::InboundHandler;
use crate::shared::SharedState;
use crate::subscriptions::dispatch;
use rekindle_utils::RandomSource;

type SharedSession = Arc<parking_lot::RwLock<Option<crate::session::Session>>>;

impl TransportNode {
    /// Start a new transport node, attach to the Veilid network, and begin
    /// dispatching inbound events to `handler`.
    ///
    /// The `session` Arc is shared with the route publisher, which
    /// republishes our personal route to the session's profile and
    /// mailbox. The session may be `None` at startup (no identity yet) —
    /// it gets populated during `IdentityCreate` or session load. `clock`
    /// is the Host's wall clock, which stamps when an own route went down.
    ///
    /// # Errors
    /// Veilid failed to start or attach.
    pub async fn start<H: InboundHandler>(
        config: TransportConfig,
        handler: Arc<H>,
        session: SharedSession,
        rng: Arc<dyn RandomSource>,
        clock: Arc<dyn rekindle_utils::Clock>,
    ) -> Result<Self> {
        info!(namespace = %config.namespace, "starting transport node");

        // The one Veilid startup path (`rekindle_protocol::node`). Knob
        // rationale (UPnP history, route-hop policy, ProtectedStore
        // workaround) lives on `VeilidStartupOptions`. The daemon's legacy
        // top-level `allow_insecure_protected_store` flag maps onto the
        // option of the same meaning.
        let node_config = rekindle_protocol::node::NodeConfig {
            storage_dir: config.storage_dir.clone(),
            app_namespace: config.namespace.clone(),
            qualifier: "rekindle".into(),
            veilid: rekindle_types::config::VeilidStartupOptions {
                allow_insecure_fallback: config.allow_insecure_protected_store
                    || config.veilid.allow_insecure_fallback,
                ..config.veilid.clone()
            },
        };
        // The dispatch loop feeds readiness from the node's attachment
        // events; the record sessions and route allocation wait on it.
        let shared_state = SharedState::new();
        let (ports, events) = rekindle_protocol::ports::VeilidPorts::start(
            &node_config,
            config.safety.dht.clone(),
            shared_state.follow_ready(),
            Arc::new(move || rekindle_utils::Clock::now_ms(clock.as_ref())),
        )
        .await?;

        let (node, shutdown_rx) = Self::outbound(config, ports, shared_state, rng, true);
        let node = node.run_inbound(shutdown_rx, events, handler, session);
        info!("transport node started");
        Ok(node)
    }

    /// A node over ports the caller built and runs (the network simulator),
    /// dispatching `events` to `handler` as [`Self::start`] does.
    pub fn from_ports<H: InboundHandler>(
        config: TransportConfig,
        ports: PortSet,
        events: mpsc::Receiver<PortEvent>,
        handler: Arc<H>,
        session: SharedSession,
        rng: Arc<dyn RandomSource>,
    ) -> Self {
        info!(namespace = %config.namespace, "transport node on supplied ports");
        let (node, shutdown_rx) = Self::outbound(config, ports, SharedState::new(), rng, true);
        node.run_inbound(shutdown_rx, events, handler, session)
    }

    /// W16.9b — a node for OUTBOUND-ONLY use over the host's ports.
    ///
    /// The host keeps its own dispatch loop (src-tauri's
    /// `lifecycle::dispatch::run_dispatch_loop`). This mode gives the caller
    /// `Sender`, `AppCaller`, the record sessions, the peer registry and the
    /// one route republisher (plan E1.1), over `session` (the Host's) —
    /// everything needed for outbound app messages, app calls, DHT
    /// operations and our routes — without consuming inbound events. The
    /// republisher has no handler to tell of route changes until E5.7,
    /// which deletes this mode when the GUI's Host starts its own node.
    pub fn adopt_outbound(config: TransportConfig, ports: PortSet, session: SharedSession) -> Self {
        info!(namespace = %config.namespace, "transport node (outbound-only) on the host's ports");
        // The host's dispatch reports peers' probes and arrivals over our
        // routes to [`Self::liveness`] (src-tauri `services::veilid::network`),
        // so this node rotates its General route on evidence as a started
        // one does (plan RV1); its jitter is OS entropy.
        let rng: Arc<dyn RandomSource> = Arc::new(rekindle_utils::OsRandom);
        let mut node = Self::outbound(config, ports, SharedState::new(), rng, true).0;
        node.spawn_route_publisher(None, session);
        node
    }

    /// The node without dispatch: senders, records, routes and peers.
    /// Every outbound `app_call` of the node is bounded here, the one place
    /// its port comes in ([`crate::broadcast::deadline`], finding 78).
    fn outbound(
        config: TransportConfig,
        mut ports: PortSet,
        shared_state: Arc<SharedState>,
        rng: Arc<dyn RandomSource>,
        sees_inbound: bool,
    ) -> (Self, mpsc::Receiver<()>) {
        ports.messages = crate::broadcast::deadline::Bounded::wrap(ports.messages);
        let config = Arc::new(config);
        let peer_registry = Arc::new(parking_lot::RwLock::new(PeerRegistry::new(
            config.route_cache_ttl_secs,
            config.circuit_breaker_threshold,
            config.circuit_breaker_cooldown_secs,
        )));
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        // Our routes are wanted from node-ready, before any login (C7.9b):
        // a login is reachable at once. Their owner waits for readiness.
        ports.routes.want_own();
        let liveness = crate::liveness::Liveness::new(rng, Arc::clone(&ports.routes), sees_inbound);
        let node = Self {
            liveness,
            liveness_handles: Vec::new(),
            ports,
            config,
            shutdown_tx,
            dispatch_handle: None,
            route_publisher_handle: None,
            route_publisher_shutdown_tx: None,
            peer_registry,
            relay: super::RelayState::default(),
            shared_state,
        };
        (node, shutdown_rx)
    }

    /// Dispatch `events` to `handler` and follow our routes.
    fn run_inbound<H: InboundHandler>(
        mut self,
        shutdown_rx: mpsc::Receiver<()>,
        events: mpsc::Receiver<PortEvent>,
        handler: Arc<H>,
        session: SharedSession,
    ) -> Self {
        self.dispatch_handle = Some(tokio::spawn(
            dispatch::run_dispatch_loop(
                Arc::clone(&handler),
                Arc::clone(&self.config),
                events,
                shutdown_rx,
                Arc::clone(&self.ports.messages),
                Arc::clone(&self.shared_state),
                dispatch::RouteOwners {
                    routes: Arc::clone(&self.ports.routes),
                    peers: Arc::clone(&self.peer_registry),
                    liveness: Arc::clone(&self.liveness),
                },
            )
            .in_current_span(),
        ));

        self.spawn_route_publisher(Some(notify_handler(handler)), session);
        self
    }

    /// Run the one route republisher over `session`.
    fn spawn_route_publisher(
        &mut self,
        on_change: Option<OnRoutesChanged>,
        session: SharedSession,
    ) {
        let (ra_tx, ra_rx) = mpsc::channel(1);
        // One watch per follower task: a tokio watch wakes its waiters in
        // an order drawn from the runtime's own RNG (tokio 1.54
        // `sync/watch.rs` `BigNotify::notified`, `thread_rng_n(8)`), not
        // the seed, so two tasks on one watch drew from the liveness RNG
        // in either order (RK1). Each gets its own, offered in turn.
        let (offer, offered) = tokio::sync::watch::channel(None);
        let mut offers = vec![offer];
        self.liveness_handles.push(tokio::spawn(
            crate::liveness::owner::run_publisher(
                Arc::clone(&self.liveness),
                offered,
                session,
                Arc::clone(&self.shared_state),
            )
            .in_current_span(),
        ));
        if self.liveness.sees_inbound() {
            let (offer, offered) = tokio::sync::watch::channel(None);
            offers.push(offer);
            self.liveness_handles.push(tokio::spawn(
                crate::liveness::owner::run_rotations(Arc::clone(&self.liveness), offered)
                    .in_current_span(),
            ));
        }
        self.route_publisher_handle = Some(tokio::spawn(
            run_route_publisher(
                on_change,
                Arc::clone(&self.ports.routes),
                self.config.route_watchdog_secs,
                ra_rx,
                offers,
            )
            .in_current_span(),
        ));
        self.route_publisher_shutdown_tx = Some(ra_tx);
    }

    /// Graceful shutdown: stop the route publisher and dispatch, release
    /// our routes, and — when this node started the network itself — detach
    /// and stop it, the dispatch loop receiving its updates through
    /// `Shutdown`. An adopted node's network is stopped by its owner.
    ///
    /// # Errors
    /// The network refused to detach; it was stopped all the same.
    pub async fn shutdown(mut self) -> Result<()> {
        info!("transport node shutting down");

        if let Some(tx) = self.route_publisher_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.route_publisher_handle.take() {
            let _ = h.await;
        }
        // The follower dropped its offers: the liveness tasks end.
        for h in std::mem::take(&mut self.liveness_handles) {
            let _ = h.await;
        }

        // Dispatch stops; the loop keeps receiving until the network's last
        // update (`Shutdown`), so nothing Veilid sends while it detaches
        // and stops is dropped on a gone consumer (live R3-6).
        let _ = self.shutdown_tx.send(()).await;

        // Release our live routes before detach, an allocation in flight
        // awaited (not cut) and released; a dead one was already forgotten
        // (plan C7.9a).
        self.ports.routes.shutdown_own().await;

        // The network is stopped even when the detach fails: a skipped
        // `shutdown()` leaves the table store and Veilid 0.5.8's namespace
        // lock held (`table_store/native.rs:113-145`), so no later start in
        // this process could open it (synthesis #12). `shutdown()` is
        // idempotent (`veilid_api/api.rs:84-104`).
        let mut detached = Ok(());
        if let Some(lifecycle) = self.ports.lifecycle.take() {
            detached = lifecycle.detach().await;
            if let Err(e) = &detached {
                warn!(error = %e, "detach failed; stopping the network anyway");
            }
            lifecycle.shutdown().await;
        }
        // The network stopped (or is its owner's): Veilid's updates are all
        // queued, `Shutdown` last (`core_context.rs:194-230`), so the drain
        // takes them without waiting and the loop ends.
        let _ = self.shutdown_tx.send(()).await;
        if let Some(handle) = self.dispatch_handle.take() {
            if let Err(e) = handle.await {
                warn!(error = %e, "dispatch loop join failed");
            }
        }

        info!("transport node shutdown complete");
        detached.map_err(|e| TransportError::ShutdownFailed {
            reason: e.to_string(),
        })
    }
}
