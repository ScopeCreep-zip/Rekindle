//! Route management — private route allocation, peer route caching,
//! route blob access for DHT publication, peer route discovery.

use rekindle_types::dht_types::PROFILE_SUBKEY_ROUTE_BLOB;

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
    /// After this call, transport.send_to_peer(peer_key, data) will use
    /// the cached route without additional DHT lookups.
    pub fn cache_peer_route(&self, peer_key: &str, route_blob: Vec<u8>) {
        self.transport().cache_peer_route(peer_key, route_blob);
    }

    /// Invalidate a cached peer route.
    pub fn invalidate_peer_route(&self, peer_key: &str) {
        self.transport().invalidate_peer_route(peer_key);
    }

    /// Allocate a new private route for receiving inbound messages.
    pub async fn allocate_route(&self) -> Result<(String, Vec<u8>), ChatError> {
        self.transport()
            .allocate_route()
            .await
            .map_err(ChatError::Transport)
    }

    /// Discover and cache a peer's route blob from their profile DHT record.
    ///
    /// Reads PROFILE_SUBKEY_ROUTE_BLOB (subkey 6) from the peer's profile,
    /// caches it in the peer registry for direct messaging, and optionally
    /// upserts the peer into a community gossip mesh.
    ///
    /// This is THE function that populates the peer registry and gossip mesh.
    /// Every code path that reads a peer's profile MUST call this afterward.
    ///
    /// `peer_key`: the peer's identity key (Ed25519 hex for friends, pseudonym hex for community members)
    /// `profile_dht_key`: the peer's profile DHT record key (VLD0:...)
    /// `mesh_info`: if Some, upserts the peer into the specified community's gossip mesh
    pub async fn discover_peer_route(
        &self,
        peer_key: &str,
        profile_dht_key: &str,
        mesh_info: Option<MeshPeerInfo<'_>>,
    ) -> Result<bool, ChatError> {
        // Read route blob from peer's profile
        let route_blob = match self.open_and_read(profile_dht_key, PROFILE_SUBKEY_ROUTE_BLOB, true).await {
            Ok(Some(blob)) if !blob.is_empty() => blob,
            Ok(_) => {
                tracing::debug!(
                    peer = &peer_key[..16.min(peer_key.len())],
                    profile = &profile_dht_key[..20.min(profile_dht_key.len())],
                    "peer has no route blob published — direct messaging unavailable"
                );
                return Ok(false);
            }
            Err(e) => {
                tracing::warn!(
                    peer = &peer_key[..16.min(peer_key.len())],
                    profile = &profile_dht_key[..20.min(profile_dht_key.len())],
                    error = %e,
                    "failed to read peer route blob"
                );
                return Ok(false);
            }
        };

        // Cache in peer registry — enables send_to_peer() and send_peer_notification()
        self.cache_peer_route(peer_key, route_blob.clone());
        tracing::info!(
            peer = &peer_key[..16.min(peer_key.len())],
            blob_len = route_blob.len(),
            "peer route cached"
        );

        // Upsert into gossip mesh if community context provided
        if let Some(info) = mesh_info {
            let now_secs = crate::time::timestamp_secs() as u64;
            self.transport().upsert_mesh_peer(
                info.community_id,
                info.pseudonym,
                route_blob,
                info.status,
                now_secs,
                info.my_pseudonym,
            );
        }

        Ok(true)
    }

    /// Upsert a peer into a community gossip mesh (without reading their profile).
    /// Used when the route blob is already known from another source (e.g., gossip payload).
    pub fn upsert_mesh_peer(
        &self,
        community_id: &str,
        pseudonym: &str,
        route_blob: Vec<u8>,
        status: &str,
        my_pseudonym: &str,
    ) {
        let now_secs = crate::time::timestamp_secs() as u64;
        self.transport().upsert_mesh_peer(community_id, pseudonym, route_blob, status, now_secs, my_pseudonym);
    }

    /// Remove a peer from a community gossip mesh.
    pub fn remove_mesh_peer(&self, community_id: &str, pseudonym: &str) {
        self.transport().remove_mesh_peer(community_id, pseudonym);
    }
}

/// Context for upserting a peer into a community gossip mesh during route discovery.
pub struct MeshPeerInfo<'a> {
    pub community_id: &'a str,
    pub pseudonym: &'a str,
    pub status: &'a str,
    pub my_pseudonym: &'a str,
}
