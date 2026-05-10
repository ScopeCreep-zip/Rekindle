//! Route management — private route allocation, peer route caching,
//! route blob access for DHT publication.

use super::PlatformIO;
use crate::ChatError;

impl PlatformIO {
    /// Get our current route blob for publishing to profile DHT.
    ///
    /// Returns None if no route has been allocated yet (daemon still
    /// in RESUMING state before route allocation completes).
    pub fn route_blob(&self) -> Option<Vec<u8>> {
        self.transport().route_blob()
    }

    /// Cache a peer's route blob for future send_to_peer / send_peer_notification calls.
    ///
    /// Called during friendship establishment when the peer's profile DHT
    /// is read and the route blob is extracted. After this call,
    /// transport.send_to_peer(peer_key, data) will use the cached route
    /// without additional DHT lookups.
    pub fn cache_peer_route(&self, peer_key: &str, route_blob: Vec<u8>) {
        self.transport().cache_peer_route(peer_key, route_blob);
    }

    /// Invalidate a cached peer route.
    ///
    /// Called when a RouteDied event indicates the peer's route is no longer
    /// valid. The next send_to_peer will fail with PeerUnreachable until
    /// the peer's profile is re-read and a fresh route is cached.
    pub fn invalidate_peer_route(&self, peer_key: &str) {
        self.transport().invalidate_peer_route(peer_key);
    }

    /// Allocate a new private route for receiving inbound messages.
    ///
    /// Returns (route_id, route_blob). The route_blob is published to
    /// profile DHT and mailbox DHT so peers can reach us. The route_id
    /// is transport-internal and not published.
    pub async fn allocate_route(&self) -> Result<(String, Vec<u8>), ChatError> {
        self.transport()
            .allocate_route()
            .await
            .map_err(ChatError::Transport)
    }
}
