use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::info;
use veilid_core::{RoutingContext, VeilidAPI, VeilidUpdate};

use crate::error::ProtocolError;
use crate::veilid_config::{build_veilid_config, VeilidStartupOptions};

/// Configuration for starting a Rekindle node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Base storage directory for Veilid stores (protected, table, block).
    pub storage_dir: String,
    /// Namespace for this application on the Veilid network.
    pub app_namespace: String,
    /// Qualifier passed to `VeilidConfig::new()` (e.g. "rekindle" or "rekindle-server").
    pub qualifier: String,
    /// Veilid startup tuning (UPnP, DHT concurrency, route hops,
    /// protected-store mode).
    pub veilid: VeilidStartupOptions,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            storage_dir: "~/.rekindle".into(),
            app_namespace: "rekindle".into(),
            qualifier: "rekindle".into(),
            veilid: VeilidStartupOptions::default(),
        }
    }
}

/// Manages the Veilid node lifecycle.
///
/// This is the core networking primitive for Rekindle. Everything — DHT,
/// messaging, presence — depends on the node being started and attached.
pub struct RekindleNode {
    config: NodeConfig,
    /// Live handle to the Veilid API (needed for private routes, shutdown, etc.).
    api: VeilidAPI,
    /// Pre-built routing context for DHT and messaging operations.
    routing_context: RoutingContext,
    /// Receiver end of the channel that carries `VeilidUpdate` events from the
    /// core callback into the application dispatch loop.
    update_rx: mpsc::Receiver<VeilidUpdate>,
}

impl RekindleNode {
    /// Start a new Veilid node and attach to the network.
    ///
    /// This will:
    /// 1. Initialize Veilid with the given config
    /// 2. Attach to the P2P network
    /// 3. Create a `RoutingContext` for DHT / messaging
    /// 4. Return the node together with a `VeilidUpdate` receiver
    pub async fn start(config: NodeConfig) -> Result<Self, ProtocolError> {
        let (update_tx, update_rx) = mpsc::channel::<VeilidUpdate>(UPDATE_QUEUE);
        let api = startup(
            &config,
            Arc::new(move |update| {
                // Non-blocking send: a full channel drops the event rather
                // than block the Veilid core thread.
                if let Err(e) = update_tx.try_send(update) {
                    let dropped = match &e {
                        mpsc::error::TrySendError::Full(u)
                        | mpsc::error::TrySendError::Closed(u) => u,
                    };
                    log_dropped(veilid_update_name(dropped));
                }
            }),
        )
        .await?;

        // A default RoutingContext (safety routing enabled).
        let routing_context = api
            .routing_context()
            .map_err(|e| ProtocolError::NodeStartup(format!("routing context: {e}")))?;

        Ok(Self {
            config,
            api,
            routing_context,
            update_rx,
        })
    }

    /// Gracefully shut down the node and disconnect from the network.
    pub async fn shutdown(self) -> Result<(), ProtocolError> {
        info!("shutting down rekindle node");
        self.api.shutdown().await;
        info!("rekindle node shut down");
        Ok(())
    }

    /// Get the node configuration.
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// Get a reference to the Veilid API handle.
    ///
    /// Used by `OwnRoutes` for private route allocation and by the importer.
    pub fn api(&self) -> &VeilidAPI {
        &self.api
    }

    /// Get a reference to the routing context (the record pool builds its
    /// own from it, with the DHT safety selection).
    pub fn routing_context(&self) -> &RoutingContext {
        &self.routing_context
    }

    /// Take ownership of the `VeilidUpdate` event receiver.
    ///
    /// The caller is expected to drive this in its own dispatch loop. The
    /// receiver is replaced with a dummy closed channel on each call, so only
    /// the first caller gets the real event stream.
    pub fn take_update_receiver(&mut self) -> mpsc::Receiver<VeilidUpdate> {
        let (_, dummy_rx) = mpsc::channel(1);
        std::mem::replace(&mut self.update_rx, dummy_rx)
    }
}

/// How many updates wait for the dispatch loop before new ones drop.
pub(crate) const UPDATE_QUEUE: usize = 4096;

/// Start the Veilid core with `callback` and attach: the one Veilid startup
/// of every host (`RekindleNode::start` for the desktop's own dispatch,
/// `ports::VeilidPorts::start` for the transport's). The callback must not
/// block the core thread.
pub(crate) async fn startup(
    config: &NodeConfig,
    callback: veilid_core::UpdateCallback,
) -> Result<VeilidAPI, ProtocolError> {
    info!(namespace = %config.app_namespace, "starting rekindle node");
    // The shared builder is the single translation of tuning knobs into
    // `VeilidConfig`; knob rationale (UPnP history, route-hop policy,
    // ProtectedStore workaround) lives on `VeilidStartupOptions` and in
    // `crate::veilid_config`.
    let veilid_config = build_veilid_config(
        &config.app_namespace,
        &config.qualifier,
        &config.storage_dir,
        &config.veilid,
    );
    let api = veilid_core::api_startup(callback, veilid_config)
        .await
        .map_err(|e| ProtocolError::NodeStartup(e.to_string()))?;
    api.attach()
        .await
        .map_err(|e| ProtocolError::AttachFailed(e.to_string()))?;
    info!("rekindle node started and attached");
    Ok(api)
}

/// Log an update the full channel dropped. During shutdown Veilid emits
/// many "Other" events, which are safe to drop, so those log at debug.
pub(crate) fn log_dropped(label: &str) {
    if label == "Other" {
        tracing::debug!(
            event = label,
            "Veilid update channel full — dropped non-critical event"
        );
    } else {
        tracing::warn!(event = label, "Veilid update channel full — dropped event");
    }
}

/// Return a human-readable name for a `VeilidUpdate` variant (for logging).
pub fn veilid_update_name(update: &VeilidUpdate) -> &'static str {
    match update {
        VeilidUpdate::AppCall(_) => "AppCall",
        VeilidUpdate::AppMessage(_) => "AppMessage",
        VeilidUpdate::RouteChange(_) => "RouteChange",
        VeilidUpdate::Attachment(_) => "Attachment",
        VeilidUpdate::ValueChange(_) => "ValueChange",
        VeilidUpdate::Shutdown => "Shutdown",
        _ => "Other",
    }
}
