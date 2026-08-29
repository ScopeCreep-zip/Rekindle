//! `DHTManager` peer-route caching: import, lookup, TTL, invalidation.

use std::time::Instant;

use veilid_core::VeilidAPI;

use super::DHTManager;
use crate::error::ProtocolError;

/// Maximum age for a cached imported route before re-importing.
///
/// Veilid routes expire after ~5 minutes. By re-importing every 90 seconds,
/// we ensure we always have a fresh `RouteId` without relying solely on
/// `RouteChange` events (which can be dropped).
const IMPORTED_ROUTE_TTL_SECS: u64 = 90;

impl DHTManager {
    /// Cache a peer's route blob for message sending.
    ///
    /// Also imports the route and records the `RouteId → pubkey` mapping
    /// so that [`invalidate_dead_routes`] can selectively remove only the
    /// affected peer's entry.
    pub fn cache_route(&mut self, api: &VeilidAPI, pubkey_hex: &str, route_blob: Vec<u8>) {
        if route_blob.is_empty() {
            tracing::debug!(peer = %pubkey_hex, "ignoring empty route blob — will fetch from DHT");
            return;
        }
        // Import and cache RouteId for selective invalidation.
        // Only cache the blob if the import succeeds — stale/expired route
        // blobs (e.g. from old invites) must not linger in the cache.
        match api.import_remote_private_route(route_blob.clone()) {
            Ok(route_id) => {
                self.imported_routes
                    .insert(route_blob.clone(), (route_id.clone(), Instant::now()));
                self.route_id_to_pubkey
                    .insert(route_id, pubkey_hex.to_string());
                self.route_cache.insert(pubkey_hex.to_string(), route_blob);
            }
            Err(e) => {
                tracing::warn!(
                    peer = %pubkey_hex,
                    error = %e,
                    blob_len = route_blob.len(),
                    route_count_byte = route_blob.first().copied().unwrap_or(0),
                    "route blob import failed — will fetch fresh route from DHT"
                );
            }
        }
    }

    /// Look up a cached route blob for a peer.
    pub fn get_cached_route(&self, pubkey_hex: &str) -> Option<&Vec<u8>> {
        self.route_cache.get(pubkey_hex)
    }

    /// Import a peer's route blob, reusing a cached `RouteId` if available.
    ///
    /// Prevents the resource leak caused by calling `import_remote_private_route`
    /// on every send — the same `RouteId` is reused until invalidated.
    pub fn get_or_import_route(
        &mut self,
        api: &VeilidAPI,
        route_blob: &[u8],
    ) -> Result<veilid_core::RouteId, ProtocolError> {
        // Check cache — evict if older than TTL
        if let Some((route_id, imported_at)) = self.imported_routes.get(route_blob) {
            if imported_at.elapsed().as_secs() < IMPORTED_ROUTE_TTL_SECS {
                return Ok(route_id.clone());
            }
            // TTL expired — remove stale entry and re-import below
            tracing::debug!("imported route TTL expired — re-importing");
            self.imported_routes.remove(route_blob);
        }
        let route_id = api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| ProtocolError::RoutingError(format!("import: {e}")))?;
        self.imported_routes
            .insert(route_blob.to_vec(), (route_id.clone(), Instant::now()));
        Ok(route_id)
    }

    /// Invalidate a cached imported route by its blob bytes.
    ///
    /// Called when a send operation fails with a route error — ensures the
    /// next `get_or_import_route` call for the same blob will perform a fresh
    /// import rather than returning the stale `RouteId`.
    pub fn invalidate_route_blob(&mut self, route_blob: &[u8]) {
        if self.imported_routes.remove(route_blob).is_some() {
            tracing::debug!(
                blob_len = route_blob.len(),
                "invalidated cached route import after send failure"
            );
        }
    }

    /// Selectively invalidate cached routes when remote routes die.
    ///
    /// Clears both the peer route cache (via `RouteId → pubkey` reverse map)
    /// and any matching entries in the `imported_routes` cache. This ensures
    /// community server routes are also invalidated when Veilid reports them
    /// as dead, not just peer-to-peer routes.
    ///
    /// Returns the pubkeys whose routes died so the caller can clear
    /// any OTHER per-peer route caches it holds (the host's live
    /// `peer_route_cache` keeps its own copy of these blobs).
    pub fn invalidate_dead_routes(&mut self, dead_routes: &[veilid_core::RouteId]) -> Vec<String> {
        let mut affected = Vec::new();
        for route_id in dead_routes {
            // Invalidate peer route cache entry
            if let Some(pubkey) = self.route_id_to_pubkey.remove(route_id) {
                tracing::debug!(
                    pubkey = %pubkey,
                    "selectively invalidating dead route for peer"
                );
                if let Some(blob) = self.route_cache.remove(&pubkey) {
                    self.imported_routes.remove(&blob);
                }
                affected.push(pubkey);
            }
        }

        // Also scan imported_routes for any matching RouteIds (covers community
        // server routes that aren't in the peer route_id_to_pubkey map).
        let dead_set: std::collections::HashSet<&veilid_core::RouteId> =
            dead_routes.iter().collect();
        self.imported_routes
            .retain(|_blob, (route_id, _ts)| !dead_set.contains(route_id));
        affected
    }

    /// Invalidate all cached route state for a peer by their public key.
    ///
    /// Removes the route blob from `route_cache`, the imported `RouteId` from
    /// `imported_routes`, and the reverse mapping from `route_id_to_pubkey`.
    /// Called after a send failure to ensure the next attempt fetches a fresh
    /// route from DHT rather than reusing the stale one.
    pub fn invalidate_route_for_peer(&mut self, pubkey_hex: &str) {
        if let Some(blob) = self.route_cache.remove(pubkey_hex) {
            if let Some((route_id, _)) = self.imported_routes.remove(&blob) {
                self.route_id_to_pubkey.remove(&route_id);
            }
            tracing::debug!(peer = %pubkey_hex, "invalidated cached route for peer after send failure");
        }
    }
}
