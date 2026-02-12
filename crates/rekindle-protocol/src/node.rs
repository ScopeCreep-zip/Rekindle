use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::info;
use veilid_core::{
    RoutingContext, VeilidAPI, VeilidConfig, VeilidUpdate,
};

use crate::error::ProtocolError;

/// Configuration for starting a Rekindle node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Base storage directory for Veilid stores (protected, table, block).
    pub storage_dir: String,
    /// Namespace for this application on the Veilid network.
    pub app_namespace: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            storage_dir: "~/.rekindle".into(),
            app_namespace: "rekindle".into(),
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
        let veilid_config = VeilidConfig::new(
            &config.app_namespace,   // program_name
            "com",                   // organization
            "rekindle",              // qualifier
            Some(&config.storage_dir), // storage_directory override
            None,                    // config_directory (use default)
        );

        // 2. Create an mpsc channel for VeilidUpdate events
        let (update_tx, update_rx) = mpsc::channel::<VeilidUpdate>(256);

        // 3. Build the update callback that forwards events into the channel
        let update_callback: veilid_core::UpdateCallback = Arc::new(move |update| {
            // Non-blocking send — if the channel is full we drop the event
            // rather than blocking the Veilid core thread.
            let _ = update_tx.try_send(update);
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
    /// The caller is expected to drive this in its own dispatch loop. This can
    /// only be called once; subsequent calls will receive `None`.
    pub fn take_update_receiver(&mut self) -> Option<mpsc::Receiver<VeilidUpdate>> {
        // We use Option trickery via std::mem::take — once taken, the field is
        // replaced with a dummy closed receiver.
        let (_, dummy_rx) = mpsc::channel(1);
        let rx = std::mem::replace(&mut self.update_rx, dummy_rx);
        Some(rx)
    }
}
