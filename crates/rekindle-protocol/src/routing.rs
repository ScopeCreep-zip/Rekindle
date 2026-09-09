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
        Self::Safe {
            hop_count: rekindle_types::config::ANONYMITY_HOP_FLOOR,
        }
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
    /// Media-class inbound route — same hop count, but allocated with
    /// `Stability::LowLatency` + `Sequencing::PreferUnordered` so the
    /// relays carrying inbound voice/video are chosen for latency and
    /// datagram capability rather than uptime and ordered dial info.
    /// See `services::veilid::network::new_media_route_with_retry`.
    media_route_id: Option<RouteId>,
    /// Blob for [`Self::media_route_id`], announced in voice joins.
    media_route_blob: Option<Vec<u8>>,
}

impl RoutingManager {
    /// Create a new `RoutingManager` backed by the given `VeilidAPI`.
    pub fn new(api: VeilidAPI, safety_mode: SafetyMode) -> Self {
        Self {
            api,
            safety_mode,
            private_route_id: None,
            private_route_blob: None,
            media_route_id: None,
            media_route_blob: None,
        }
    }

    /// Release (destroy) the current private route.
    ///
    /// Calls the Veilid API to free the route. If the route has already expired
    /// or been cleaned up by Veilid (e.g. reported via `RouteChange`), this is
    /// a no-op — use [`forget_private_route`] instead when the route is known
    /// to be dead.
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

    /// Clear the current private route from our state without calling Veilid.
    ///
    /// Used when Veilid has already reported the route as dead via a
    /// `RouteChange` event — calling `release_private_route` on a dead route
    /// produces an "Invalid argument" error from the Veilid API.
    pub fn forget_private_route(&mut self) {
        self.private_route_id = None;
        self.private_route_blob = None;
        tracing::info!("private route forgotten (already dead)");
    }

    /// Import a remote peer's private route from their route blob.
    ///
    /// Returns the `RouteId` as a string so it can be used as a target for
    /// `app_message` / `app_call`.
    pub fn import_route(&self, route_blob: &[u8]) -> Result<String, ProtocolError> {
        let route_id = self
            .api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| {
                ProtocolError::RoutingError(format!("import_remote_private_route: {e}"))
            })?;

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

    /// Our media-class route blob, if one has been allocated.
    ///
    /// Callers fall back to [`Self::route_blob`] when this is `None`:
    /// a general route still carries media, just over relays picked for
    /// the wrong thing.
    pub fn media_route_blob(&self) -> Option<&Vec<u8>> {
        self.media_route_blob.as_ref()
    }

    /// Install an externally-allocated media-class route.
    pub fn set_allocated_media_route(&mut self, route_id: RouteId, blob: Vec<u8>) {
        self.media_route_id = Some(route_id);
        self.media_route_blob = Some(blob);
    }

    /// Release the media-class route, if allocated.
    ///
    /// Separate from [`Self::release_private_route`] because the two
    /// have independent lifetimes: a dead media route must not take the
    /// general route down with it.
    pub fn release_media_route(&mut self) -> Result<(), ProtocolError> {
        if let Some(route_id) = self.media_route_id.take() {
            self.api
                .release_private_route(route_id)
                .map_err(|e| ProtocolError::RoutingError(format!("release_media_route: {e}")))?;
        }
        self.media_route_blob = None;
        Ok(())
    }

    /// Install the route from an externally-allocated `RouteBlob`.
    ///
    /// Used when the caller needs to call `api.new_private_route()` outside
    /// of a lock guard (e.g. `parking_lot` across an `.await` boundary) and
    /// then store the result back.
    ///
    /// Routes are event-driven (no proactive rotation): install happens
    /// only at first login or after a dead route was `forget`-cleared, so
    /// there is never a live route to replace — a plain install.
    pub fn set_allocated_route(&mut self, route_id: RouteId, blob: Vec<u8>) {
        self.private_route_id = Some(route_id);
        self.private_route_blob = Some(blob);
    }
}
