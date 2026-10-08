//! Transport node startup, adoption, and shutdown.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{info, warn};
use veilid_core::{VeilidAPI, VeilidUpdate};

use super::routes::run_route_publisher;
use super::TransportNode;
use crate::broadcast::peer_registry::PeerRegistry;
use crate::config::TransportConfig;
use crate::error::{Result, TransportError};
use crate::handler::InboundHandler;
use crate::shared::SharedState;
use crate::subscriptions::dispatch;
use rekindle_protocol::node::veilid_update_name as veilid_update_label;

impl TransportNode {
    /// Start a new transport node, attach to the Veilid network, and begin
    /// dispatching inbound events to `handler`.
    ///
    /// The `session` Arc is shared with the route authority loop so
    /// dead community-mailbox routes heal alongside the personal route.
    /// The session may be `None` at startup (no identity yet) — it gets
    /// populated during `IdentityCreate` or session load.
    pub async fn start<H: InboundHandler>(
        config: TransportConfig,
        handler: Arc<H>,
        session: Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    ) -> Result<Self> {
        info!(namespace = %config.namespace, "starting transport node");

        // Build VeilidConfig via the shared builder — the single
        // translation of tuning knobs into `VeilidConfig` for both
        // node-startup tracks (`rekindle_protocol::veilid_config`). Knob
        // rationale (UPnP history, route-hop policy, ProtectedStore
        // workaround) lives on `VeilidStartupOptions`. The daemon's
        // legacy top-level `allow_insecure_protected_store` flag maps
        // onto the option of the same meaning.
        let veilid_options = rekindle_types::config::VeilidStartupOptions {
            allow_insecure_fallback: config.allow_insecure_protected_store
                || config.veilid.allow_insecure_fallback,
            ..config.veilid.clone()
        };
        let veilid_config = rekindle_protocol::veilid_config::build_veilid_config(
            &config.namespace,
            "rekindle",
            &config.storage_dir,
            &veilid_options,
        );

        let (update_tx, update_rx) = mpsc::channel::<VeilidUpdate>(4096);
        let update_callback: veilid_core::UpdateCallback = Arc::new(move |update| {
            if let Err(e) = update_tx.try_send(update) {
                let label = match &e {
                    mpsc::error::TrySendError::Full(u) | mpsc::error::TrySendError::Closed(u) => {
                        veilid_update_label(u)
                    }
                };
                if label == "Other" {
                    tracing::debug!("veilid update channel full — dropped non-critical event");
                } else {
                    tracing::warn!(event = label, "veilid update channel full — dropped event");
                }
            }
        });

        let api = veilid_core::api_startup(update_callback, veilid_config)
            .await
            .map_err(|e| TransportError::AttachFailed {
                reason: format!("api_startup: {e}"),
            })?;

        api.attach()
            .await
            .map_err(|e| TransportError::AttachFailed {
                reason: format!("attach: {e}"),
            })?;

        let config = Arc::new(config);
        let peer_registry = Arc::new(parking_lot::RwLock::new(PeerRegistry::new(
            config.route_cache_ttl_secs,
            config.circuit_breaker_threshold,
            config.circuit_breaker_cooldown_secs,
        )));
        let shared_state = SharedState::new();

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let own_routes = rekindle_protocol::own_routes::OwnRoutes::new(
            rekindle_protocol::own_routes::VeilidRouteAllocator::new(api.clone()),
            shared_state.subscribe_ready(),
        );
        let route_imports = Arc::new(rekindle_protocol::dht::route_imports::RouteImports::new(
            api.clone(),
        ));
        let dispatch_handle = tokio::spawn(dispatch::run_dispatch_loop(
            Arc::clone(&handler),
            Arc::clone(&config),
            update_rx,
            shutdown_rx,
            api.clone(),
            Arc::clone(&shared_state),
            dispatch::RouteOwners {
                own: Arc::clone(&own_routes),
                imports: Arc::clone(&route_imports),
                peers: Arc::clone(&peer_registry),
            },
        ));

        let (ra_tx, ra_rx) = mpsc::channel(1);
        let route_publisher_handle = tokio::spawn(run_route_publisher(
            Arc::clone(&handler),
            Arc::clone(&own_routes),
            config.route_watchdog_secs,
            ra_rx,
            Arc::clone(&session),
            Arc::clone(&shared_state),
        ));

        info!("transport node started");

        Ok(Self {
            route_imports,
            record_closer: rekindle_protocol::dht::pool::RecordCloser::new(),
            api,
            config,
            shutdown_tx,
            dispatch_handle: Some(dispatch_handle),
            route_publisher_handle: Some(route_publisher_handle),
            route_publisher_shutdown_tx: Some(ra_tx),
            own_routes: Some(own_routes),
            peer_registry,
            shared_state,
        })
    }

    /// W16.9 — adopt an EXTERNALLY-MANAGED Veilid API + update channel.
    ///
    /// Use this when the host process already owns Veilid bootstrap
    /// (e.g. `src-tauri/src/services/veilid/lifecycle/node.rs` runs
    /// `api_startup` + `attach` itself for historical reasons). The
    /// transport adopts the running `VeilidAPI` and the `Receiver` end
    /// of the host's existing update channel, spawns its own dispatch
    /// loop on top, and otherwise behaves identically to [`Self::start`].
    ///
    /// **Caller MUST stop calling its own dispatch loop on the same
    /// `update_rx`** — both consumers can't share a single mpsc receiver.
    /// The transport now owns the dispatch.
    ///
    /// On [`Self::shutdown`], `adopt`-ed nodes do NOT detach or shut
    /// down Veilid (the host owns lifecycle). Pass `owns_veilid: false`
    /// to skip the api.detach()/api.shutdown() steps.
    pub fn adopt<H: InboundHandler>(
        config: TransportConfig,
        api: VeilidAPI,
        update_rx: mpsc::Receiver<VeilidUpdate>,
        handler: &Arc<H>,
        session: &Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    ) -> Self {
        info!(namespace = %config.namespace, "adopting external Veilid node into transport");

        let config = Arc::new(config);
        let peer_registry = Arc::new(parking_lot::RwLock::new(PeerRegistry::new(
            config.route_cache_ttl_secs,
            config.circuit_breaker_threshold,
            config.circuit_breaker_cooldown_secs,
        )));
        let shared_state = SharedState::new();

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let own_routes = rekindle_protocol::own_routes::OwnRoutes::new(
            rekindle_protocol::own_routes::VeilidRouteAllocator::new(api.clone()),
            shared_state.subscribe_ready(),
        );
        let route_imports = Arc::new(rekindle_protocol::dht::route_imports::RouteImports::new(
            api.clone(),
        ));
        let dispatch_handle = tokio::spawn(dispatch::run_dispatch_loop(
            Arc::clone(handler),
            Arc::clone(&config),
            update_rx,
            shutdown_rx,
            api.clone(),
            Arc::clone(&shared_state),
            dispatch::RouteOwners {
                own: Arc::clone(&own_routes),
                imports: Arc::clone(&route_imports),
                peers: Arc::clone(&peer_registry),
            },
        ));

        let (ra_tx, ra_rx) = mpsc::channel(1);
        let route_publisher_handle = tokio::spawn(run_route_publisher(
            Arc::clone(handler),
            Arc::clone(&own_routes),
            config.route_watchdog_secs,
            ra_rx,
            Arc::clone(session),
            Arc::clone(&shared_state),
        ));

        info!("transport node adopted (host owns Veilid lifecycle)");

        Self {
            route_imports,
            record_closer: rekindle_protocol::dht::pool::RecordCloser::new(),
            api,
            config,
            shutdown_tx,
            dispatch_handle: Some(dispatch_handle),
            route_publisher_handle: Some(route_publisher_handle),
            route_publisher_shutdown_tx: Some(ra_tx),
            own_routes: Some(own_routes),
            peer_registry,
            shared_state,
        }
    }

    /// W16.9b — adopt an externally-managed Veilid API for OUTBOUND-ONLY use.
    ///
    /// The host process keeps its own dispatch loop on `update_rx` (e.g.
    /// src-tauri's `lifecycle::dispatch::run_dispatch_loop`). This mode
    /// gives the caller `Sender`, `Caller`, the record pool, and the peer
    /// registry — everything needed for outbound `app_message`,
    /// `app_call`, and DHT operations — without consuming inbound
    /// updates. Useful during migration: the host's existing
    /// receive-side service code keeps handling inbound events while
    /// new send paths route through transport's typed APIs (e.g.
    /// `operations::friend::send_friend_request`, `operations::dm_invite`).
    ///
    /// NO route owner is built in this mode: the HOST owns its routes
    /// (src-tauri's `OwnRoutes` and `route_publish`). Outbound safety
    /// routes are allocated internally by veilid-core per send.
    pub fn adopt_outbound(
        config: TransportConfig,
        api: VeilidAPI,
        session: &Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    ) -> Self {
        info!(namespace = %config.namespace, "adopting Veilid (outbound-only) into transport");

        let config = Arc::new(config);
        let peer_registry = Arc::new(parking_lot::RwLock::new(PeerRegistry::new(
            config.route_cache_ttl_secs,
            config.circuit_breaker_threshold,
            config.circuit_breaker_cooldown_secs,
        )));
        let shared_state = SharedState::new();

        // Send-side only — no dispatch loop, no route loop (host owns
        // both the dispatch and the route lifecycle in this mode).
        let (shutdown_tx, _shutdown_rx) = mpsc::channel(1);
        let _ = session; // host owns session-derived publishing too

        info!("transport node adopted (outbound-only — host owns dispatch + Veilid lifecycle)");

        Self {
            route_imports: Arc::new(rekindle_protocol::dht::route_imports::RouteImports::new(
                api.clone(),
            )),
            record_closer: rekindle_protocol::dht::pool::RecordCloser::new(),
            api,
            config,
            shutdown_tx,
            dispatch_handle: None,
            route_publisher_handle: None,
            route_publisher_shutdown_tx: None,
            own_routes: None,
            peer_registry,
            shared_state,
        }
    }

    /// Shutdown without detaching/teardown of Veilid (for `adopt`-ed
    /// nodes — the host process owns Veilid lifecycle and will detach
    /// itself).
    pub async fn shutdown_borrowed(mut self) -> Result<()> {
        info!("transport node (borrowed) shutting down dispatch + route publisher");
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
        // The routes are this transport's even though Veilid is the host's.
        if let Some(own_routes) = self.own_routes.take() {
            own_routes.shutdown().await;
        }
        info!("transport node (borrowed) dispatch + route publisher stopped");
        Ok(())
    }

    /// Graceful shutdown: signal all background tasks, detach, shut down Veilid.
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
        if let Some(own_routes) = self.own_routes.take() {
            own_routes.shutdown().await;
        }

        self.api
            .detach()
            .await
            .map_err(|e| TransportError::ShutdownFailed {
                reason: format!("detach: {e}"),
            })?;
        self.api.shutdown().await;

        info!("transport node shutdown complete");
        Ok(())
    }
}
