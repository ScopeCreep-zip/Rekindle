//! Private route lifecycle management.
//!
//! Handles allocation, refresh, release, and dead-route recovery for
//! the local node's inbound private route.

use veilid_core::RouteId;

/// Manages the local node's private route for receiving messages.
///
/// The route blob is published to DHT (profile + mailbox) so peers
/// can reach us. Routes expire naturally; the [`RouteManager`] tracks
/// the current allocation and supports refresh/replacement.
pub struct RouteManager {
    /// Currently allocated route ID.
    route_id: Option<RouteId>,
    /// Currently allocated route blob (shared with peers).
    route_blob: Option<Vec<u8>>,
}

impl RouteManager {
    pub fn new() -> Self {
        Self {
            route_id: None,
            route_blob: None,
        }
    }

    /// Store a newly allocated route.
    pub fn set_route(&mut self, route_id: RouteId, blob: Vec<u8>) {
        self.route_id = Some(route_id);
        self.route_blob = Some(blob);
    }

    /// Get the current route blob for publishing to DHT.
    pub fn route_blob(&self) -> Option<&[u8]> {
        self.route_blob.as_deref()
    }

    /// Get the current route ID.
    pub fn route_id(&self) -> Option<&RouteId> {
        self.route_id.as_ref()
    }

    /// Clear the current route without releasing it via Veilid.
    ///
    /// Used when Veilid has already reported the route as dead via a
    /// `RouteChange` event — calling `release_private_route` on a dead
    /// route produces an error.
    pub fn forget_route(&mut self) {
        self.route_id = None;
        self.route_blob = None;
        tracing::info!("private route forgotten (already dead)");
    }

    /// Whether we have an active route.
    pub fn has_route(&self) -> bool {
        self.route_id.is_some()
    }
}

impl Default for RouteManager {
    fn default() -> Self {
        Self::new()
    }
}
