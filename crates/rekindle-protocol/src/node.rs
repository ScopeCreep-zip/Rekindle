use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::info;
use veilid_core::{RoutingContext, VeilidAPI, VeilidConfig, VeilidUpdate};

use crate::error::ProtocolError;

/// Configuration for starting a Rekindle node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Base storage directory for Veilid stores (protected, table, block).
    pub storage_dir: String,
    /// Namespace for this application on the Veilid network.
    pub app_namespace: String,
    /// Qualifier passed to `VeilidConfig::new()` (e.g. "rekindle" or "rekindle-server").
    pub qualifier: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            storage_dir: "~/.rekindle".into(),
            app_namespace: "rekindle".into(),
            qualifier: "rekindle".into(),
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
        info!(
            namespace = %config.app_namespace,
            "starting rekindle node"
        );

        // 1. Build VeilidConfig from our NodeConfig
        //
        // `network.rpc.default_route_hop_count` stays at the veilid
        // default (1): inbound private routes are 1-hop, the Safe send
        // side is 3-hop, so every compiled path is safety(3)+private(1)
        // = 4 hops — at/above the architecture §8 "Compiled Route =
        // Safety + Private" 3-hop target. Pinning inbound routes to 3
        // hops was tried and reverted: `new_private_route()` round-trip
        // TESTS each allocation, so 3-hop tripled the relays that must
        // all answer (allocations flapped, presence rows published
        // empty blobs) and put 6 hops under every voice frame — voice
        // rosters stopped forming. Reply-path safety for inbound RPCs
        // is handled by the one-cycle route-release grace instead.
        let mut veilid_config = VeilidConfig::new(
            &config.app_namespace,     // program_name
            "com",                     // organization
            &config.qualifier,         // qualifier
            Some(&config.storage_dir), // storage_directory override
            None,                      // config_directory (use default)
        );
        // veilid-core 0.5.3's ProtectedStore::init reaches for the OS keyring
        // (keyring-manager 0.7.1). On Linux that goes through
        // secret-service's BLOCKING zbus D-Bus API, which does
        // `Runtime::block_on` — and we call `api_startup().await` from inside
        // our Tokio runtime, so it panics ("Cannot start a runtime from within
        // a runtime"). `allow_insecure_fallback` does NOT help: `new_secure()`
        // is called (and panics) before any fallback runs. We don't need
        // veilid's keyring at all — it only protects veilid's own node/route
        // secrets, which live in our `storage_dir`; our user identity keys are
        // in the SQLCipher vault. So always use insecure (file) storage. This
        // is a config value (cross-platform-uniform, no OS branching).
        veilid_config.protected_store.always_use_insecure_storage = true;

        // Disable UPnP/IGD automatic port mapping. This sidesteps a latent
        // veilid-core panic (present in 0.5.2 AND 0.5.3, fixed only on
        // unreleased git main): when `upnp_task` can't reach an IGD gateway it
        // logs "upnp failed, restarting local network" and sets
        // `network_needs_restart`, which detaches and re-runs
        // `Network::startup_internal`. That second startup calls
        // `refresh_network_state().await?.unwrap_or_log()`
        // (native/mod.rs:750), but `refresh_network_state` returns `Ok(None)`
        // whenever the interfaces are UNCHANGED — which is exactly the case for
        // a UPnP-triggered restart — so the unwrap panics and the node is left
        // permanently detached. With `upnp = false` the task is never ticked
        // (`if upnp { upnp_task.tick() }`, native/tasks/mod.rs:140), so the
        // trigger never fires. UPnP is only an inbound-reachability
        // optimization: a NAT'd node without a mapped port falls back to
        // inbound relays via VICE (Veilid developer book → NAT Traversal), so
        // disabling it degrades gracefully to relay-based inbound rather than
        // breaking connectivity. Cross-platform-uniform (the bug is in the
        // native backend on all of Linux/macOS/Windows); no OS branching.
        veilid_config.network.upnp = false;

        // 2. Create an mpsc channel for VeilidUpdate events
        let (update_tx, update_rx) = mpsc::channel::<VeilidUpdate>(4096);

        // 3. Build the update callback that forwards events into the channel
        let update_callback: veilid_core::UpdateCallback = Arc::new(move |update| {
            // Non-blocking send — if the channel is full we drop the event
            // rather than blocking the Veilid core thread.
            if let Err(e) = update_tx.try_send(update) {
                let dropped = match &e {
                    mpsc::error::TrySendError::Full(u) | mpsc::error::TrySendError::Closed(u) => u,
                };
                // During shutdown Veilid emits many "Other" events which are
                // safe to drop — log those at debug, everything else at warn.
                if veilid_update_name(dropped) == "Other" {
                    tracing::debug!(
                        event = "Other",
                        "Veilid update channel full — dropped non-critical event"
                    );
                } else {
                    tracing::warn!(
                        event = veilid_update_name(dropped),
                        "Veilid update channel full — dropped event"
                    );
                }
            }
        });

        // 4. Start the Veilid core
        let api = veilid_core::api_startup(update_callback, veilid_config)
            .await
            .map_err(|e| ProtocolError::NodeStartup(e.to_string()))?;

        // 5. Attach to the P2P network
        api.attach()
            .await
            .map_err(|e| ProtocolError::AttachFailed(e.to_string()))?;

        // 6. Obtain a default RoutingContext (safety routing enabled)
        let routing_context = api
            .routing_context()
            .map_err(|e| ProtocolError::NodeStartup(format!("routing context: {e}")))?;

        info!("rekindle node started and attached");

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
    /// Used by `RoutingManager` for private route allocation/import.
    pub fn api(&self) -> &VeilidAPI {
        &self.api
    }

    /// Get a reference to the routing context.
    ///
    /// Used by `DHTManager` for record CRUD operations.
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
