use veilid_core::{RouteId, VeilidAPI};

use crate::error::ProtocolError;

/// Safety selection for Veilid routing.
///
/// Controls the privacy/performance tradeoff:
/// - `Safe` routes through safety nodes (sender privacy, higher latency)
/// - `Unsafe` uses direct connections (no sender privacy, lower latency)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyMode {
    /// Route through safety nodes for sender anonymity.
    Safe { hop_count: u8 },
    /// Direct connection (used for voice to minimize latency).
    Unsafe,
}

impl Default for SafetyMode {
    fn default() -> Self {
        Self::Safe { hop_count: 1 }
    }
}

/// Manages the Veilid `RoutingContext` and private route lifecycle.
pub struct RoutingManager {
    /// Veilid API handle — needed for private route allocation / import.
    api: VeilidAPI,
    /// Current safety mode for message sending.
    pub safety_mode: SafetyMode,
    /// Our allocated private route ID (typed Veilid `RouteId`).
    private_route_id: Option<RouteId>,
    /// Our private route blob (shared with peers for receiving messages).
    pub private_route_blob: Option<Vec<u8>>,
}

impl RoutingManager {
    /// Create a new `RoutingManager` backed by the given `VeilidAPI`.
    pub fn new(api: VeilidAPI, safety_mode: SafetyMode) -> Self {
        Self {
            api,
            safety_mode,
            private_route_id: None,
            private_route_blob: None,
        }
    }

    /// Allocate a new private route for receiving messages.
    ///
    /// Returns the route blob that should be published to the DHT so peers
    /// can send us messages.
    pub async fn allocate_private_route(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let route_blob = self
            .api
            .new_private_route()
            .await
            .map_err(|e| ProtocolError::RoutingError(format!("new_private_route: {e}")))?;

        self.private_route_id = Some(route_blob.route_id.clone());
        self.private_route_blob = Some(route_blob.blob.clone());

        tracing::info!(
            route_id = %route_blob.route_id,
            blob_len = route_blob.blob.len(),
            "private route allocated"
        );

        Ok(route_blob.blob)
    }

    /// Release (destroy) the current private route.
    pub fn release_private_route(&mut self) -> Result<(), ProtocolError> {
        if let Some(route_id) = self.private_route_id.take() {
            self.api
                .release_private_route(route_id)
                .map_err(|e| ProtocolError::RoutingError(format!("release_private_route: {e}")))?;
        }
        self.private_route_blob = None;
        tracing::info!("private route released");
        Ok(())
    }

    /// Import a remote peer's private route from their route blob.
    ///
    /// Returns the `RouteId` as a string so it can be used as a target for
    /// `app_message` / `app_call`.
    pub fn import_route(&self, route_blob: &[u8]) -> Result<String, ProtocolError> {
        let route_id = self
            .api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| ProtocolError::RoutingError(format!("import_remote_private_route: {e}")))?;

        Ok(route_id.to_string())
    }

    /// Get our current route blob for publishing to DHT.
    pub fn route_blob(&self) -> Option<&Vec<u8>> {
        self.private_route_blob.as_ref()
    }

    /// Get our current private route ID (if allocated).
    pub fn route_id(&self) -> Option<RouteId> {
        self.private_route_id.clone()
    }

    /// Set the route from an externally-allocated `RouteBlob`.
    ///
    /// Used when the caller needs to call `api.new_private_route()` outside
    /// of a lock guard (e.g. `parking_lot` across an `.await` boundary) and
    /// then store the result back.
    pub fn set_allocated_route(&mut self, route_id: RouteId, blob: Vec<u8>) {
        self.private_route_id = Some(route_id);
        self.private_route_blob = Some(blob);
    }
}
