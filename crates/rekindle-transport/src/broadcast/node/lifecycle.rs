//! Transport node startup, adoption, and shutdown.

use std::sync::Arc;

use rekindle_records::port::{PortEvent, PortSet};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::routes::{notify_handler, run_route_publisher, OnRoutesChanged};
use super::TransportNode;
use crate::broadcast::peer_registry::PeerRegistry;
use crate::config::TransportConfig;
use crate::error::{Result, TransportError};
use crate::handler::InboundHandler;
use crate::shared::SharedState;
use crate::subscriptions::dispatch;

type SharedSession = Arc<parking_lot::RwLock<Option<crate::session::Session>>>;

impl TransportNode {
    /// Start a new transport node, attach to the Veilid network, and begin
    /// dispatching inbound events to `handler`.
    ///
    /// The `session` Arc is shared with the route publisher, which
    /// republishes our personal route to the session's profile and
    /// mailbox. The session may be `None` at startup (no identity yet) —
    /// it gets populated during `IdentityCreate` or session load.
    ///
    /// # Errors
    /// Veilid failed to start or attach.
    pub async fn start<H: InboundHandler>(
        config: TransportConfig,
        handler: Arc<H>,
        session: SharedSession,
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
            shared_state.subscribe_ready(),
        )
        .await?;

        let (node, shutdown_rx) = Self::outbound(config, ports, shared_state);
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
    ) -> Self {
        info!(namespace = %config.namespace, "transport node on supplied ports");
        let (node, shutdown_rx) = Self::outbound(config, ports, SharedState::new());
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
        let mut node = Self::outbound(config, ports, SharedState::new()).0;
        node.spawn_route_publisher(None, session);
        node
    }

    /// The node without dispatch: senders, records, routes and peers.
    fn outbound(
        config: TransportConfig,
        ports: PortSet,
        shared_state: Arc<SharedState>,
    ) -> (Self, mpsc::Receiver<()>) {
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
        let node = Self {
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
        self.dispatch_handle = Some(tokio::spawn(dispatch::run_dispatch_loop(
            Arc::clone(&handler),
            Arc::clone(&self.config),
            events,
            shutdown_rx,
            Arc::clone(&self.ports.messages),
            Arc::clone(&self.shared_state),
            dispatch::RouteOwners {
                routes: Arc::clone(&self.ports.routes),
                peers: Arc::clone(&self.peer_registry),
            },
        )));

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
        self.route_publisher_handle = Some(tokio::spawn(run_route_publisher(
            on_change,
            Arc::clone(&self.ports.routes),
            self.config.route_watchdog_secs,
            ra_rx,
            session,
            Arc::clone(&self.shared_state),
        )));
        self.route_publisher_shutdown_tx = Some(ra_tx);
    }

    /// Graceful shutdown: stop the route publisher and dispatch, release
    /// our routes, and — when this node started the network itself — detach
    /// and stop it. An adopted node's network is stopped by its owner.
    ///
    /// # Errors
    /// The network refused to detach.
    pub async fn shutdown(mut self) -> Result<()> {
        info!("transport node shutting down");

        if let Some(tx) = self.route_publisher_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.route_publisher_handle.take() {
            let _ = h.await;
        }

        let _ = self.shutdown_tx.send(()).await;
        if let Some(handle) = self.dispatch_handle.take() {
            if let Err(e) = handle.await {
                warn!(error = %e, "dispatch loop join failed");
            }
        }

        // Release our live routes before detach; a dead one was already
        // forgotten (plan C7.9a).
        self.ports.routes.shutdown_own().await;

        if let Some(lifecycle) = self.ports.lifecycle.take() {
            lifecycle
                .detach()
                .await
                .map_err(|e| TransportError::ShutdownFailed {
                    reason: e.to_string(),
                })?;
            lifecycle.shutdown().await;
        }

        info!("transport node shutdown complete");
        Ok(())
    }
}
