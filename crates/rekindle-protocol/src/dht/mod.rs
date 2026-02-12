pub mod account;
pub mod channel;
pub mod community;
pub mod conversation;
pub mod friends;
pub mod log;
pub mod presence;
pub mod profile;
pub mod short_array;

use std::collections::{HashMap, HashSet};

use veilid_core::{
    DHTSchema, RoutingContext, ValueSubkeyRangeSet, VeilidAPI, CRYPTO_KIND_VLD0,
};

use crate::error::ProtocolError;

/// Manages DHT record operations (profiles, friend lists, communities).
///
/// Wraps a Veilid `RoutingContext` to perform record CRUD, watch, and get/set
/// operations on the distributed hash table.
pub struct DHTManager {
    /// Veilid routing context used for all DHT operations.
    routing_context: RoutingContext,
    /// Our own profile record key.
    pub profile_key: Option<String>,
    /// Our friend list record key.
    pub friend_list_key: Option<String>,
    /// Cached route blobs for known peers (`pubkey_hex` -> `route_blob`).
    pub route_cache: HashMap<String, Vec<u8>>,
    /// All record keys opened/created in this session, for bulk close on shutdown.
    pub open_records: HashSet<String>,
    /// Cache of imported route blobs → `RouteId` to prevent resource leaks.
    /// Without this, each call to `import_remote_private_route` leaks a `RouteId`.
    pub imported_routes: HashMap<Vec<u8>, veilid_core::RouteId>,
    /// Reverse map from `RouteId` → pubkey hex for selective invalidation.
    pub route_id_to_pubkey: HashMap<veilid_core::RouteId, String>,
}

impl DHTManager {
    /// Create a new `DHTManager` backed by the given `RoutingContext`.
    pub fn new(routing_context: RoutingContext) -> Self {
        Self {
            routing_context,
            profile_key: None,
            friend_list_key: None,
            route_cache: HashMap::new(),
            open_records: HashSet::new(),
            imported_routes: HashMap::new(),
            route_id_to_pubkey: HashMap::new(),
        }
    }

    /// Create a new DHT record with DFLT schema (single owner).
    ///
    /// Returns `(record_key, owner_keypair)`. The `owner_keypair` is the randomly
    /// generated keypair that owns this record — it **must** be persisted and passed
    /// back to [`open_record_writable`] on subsequent sessions to retain write access.
    pub async fn create_record(
        &self,
        subkey_count: u32,
    ) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
        let count = u16::try_from(subkey_count)
            .map_err(|_| ProtocolError::DhtError(format!("subkey_count {subkey_count} exceeds u16::MAX")))?;
        let schema = DHTSchema::dflt(count)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;

        let descriptor = self
            .routing_context
            .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("create_dht_record: {e}")))?;

        let key_string = descriptor.key().to_string();
        // Extract the owner keypair so the caller can persist it for future writes.
        // owner_secret() is Some immediately after create — Veilid generates a random
        // keypair and passes it as the writer.
        let owner_keypair = descriptor
            .owner_secret()
            .map(|secret| veilid_core::KeyPair::new_from_parts(descriptor.owner().clone(), secret.value()));

        tracing::debug!(key = %key_string, has_keypair = owner_keypair.is_some(), "created DHT record");
        Ok((key_string, owner_keypair))
    }

    /// Open an existing DHT record for **reading only** (no writer set).
    ///
    /// Use [`open_record_writable`] instead when you need to write to records you own.
    pub async fn open_record(&self, key: &str) -> Result<(), ProtocolError> {
        let record_key = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))?;

        let _descriptor = self
            .routing_context
            .open_dht_record(record_key, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("open_dht_record: {e}")))?;

        tracing::debug!(key, "opened DHT record (read-only)");
        Ok(())
    }

    /// Open an existing DHT record **with write access** by providing the owner keypair.
    ///
    /// The `writer` must be the same keypair returned by [`create_record`] when the
    /// record was originally created. Without it, Veilid's `set_dht_value` will fail
    /// with "value is not writable".
    pub async fn open_record_writable(
        &self,
        key: &str,
        writer: veilid_core::KeyPair,
    ) -> Result<(), ProtocolError> {
        let record_key = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))?;

        let _descriptor = self
            .routing_context
            .open_dht_record(record_key, Some(writer))
            .await
            .map_err(|e| ProtocolError::DhtError(format!("open_dht_record (writable): {e}")))?;

        tracing::debug!(key, "opened DHT record (writable)");
        Ok(())
    }

    /// Close a DHT record.
    pub async fn close_record(&self, key: &str) -> Result<(), ProtocolError> {
        let record_key = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))?;

        self.routing_context
            .close_dht_record(record_key)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("close_dht_record: {e}")))?;

        tracing::debug!(key, "closed DHT record");
        Ok(())
    }

    /// Get a subkey value from a DHT record.
    ///
    /// Returns `None` if the subkey has not been set yet.
    pub async fn get_value(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<Vec<u8>>, ProtocolError> {
        let record_key = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))?;

        let value = self
            .routing_context
            .get_dht_value(record_key, subkey, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("get_dht_value: {e}")))?;

        Ok(value.map(|v| v.data().to_vec()))
    }

    /// Set a subkey value on a DHT record we own.
    pub async fn set_value(
        &self,
        key: &str,
        subkey: u32,
        value: Vec<u8>,
    ) -> Result<(), ProtocolError> {
        let record_key = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))?;

        self.routing_context
            .set_dht_value(record_key, subkey, value, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("set_dht_value: {e}")))?;

        Ok(())
    }

    /// Watch specific subkeys on a DHT record for changes.
    ///
    /// Returns `true` if the watch is active, `false` if it was cancelled.
    pub async fn watch_record(&self, key: &str, subkeys: &[u32]) -> Result<bool, ProtocolError> {
        let record_key = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))?;

        // Build a ValueSubkeyRangeSet from the provided subkey indices
        let subkey_range: ValueSubkeyRangeSet = subkeys.iter().copied().collect();

        let active = self
            .routing_context
            .watch_dht_values(record_key, Some(subkey_range), None, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("watch_dht_values: {e}")))?;

        tracing::debug!(key, ?subkeys, active, "watching DHT record");
        Ok(active)
    }

    /// Cache a peer's route blob for message sending.
    ///
    /// Also imports the route and records the `RouteId → pubkey` mapping
    /// so that [`invalidate_dead_routes`] can selectively remove only the
    /// affected peer's entry.
    pub fn cache_route(&mut self, api: &VeilidAPI, pubkey_hex: &str, route_blob: Vec<u8>) {
        // Import and cache RouteId for selective invalidation
        if let Ok(route_id) = api.import_remote_private_route(route_blob.clone()) {
            self.imported_routes
                .insert(route_blob.clone(), route_id.clone());
            self.route_id_to_pubkey
                .insert(route_id, pubkey_hex.to_string());
        }
        self.route_cache
            .insert(pubkey_hex.to_string(), route_blob);
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
        if let Some(route_id) = self.imported_routes.get(route_blob) {
            return Ok(route_id.clone());
        }
        let route_id = api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| ProtocolError::RoutingError(format!("import: {e}")))?;
        self.imported_routes
            .insert(route_blob.to_vec(), route_id.clone());
        Ok(route_id)
    }

    /// Selectively invalidate cached routes when remote routes die.
    ///
    /// Uses the `RouteId → pubkey` reverse map to remove only the affected
    /// peer's cache entries, rather than clearing everything.
    pub fn invalidate_dead_routes(
        &mut self,
        api: &VeilidAPI,
        dead_routes: &[veilid_core::RouteId],
    ) {
        for route_id in dead_routes {
            if let Some(pubkey) = self.route_id_to_pubkey.remove(route_id) {
                tracing::debug!(
                    pubkey = %pubkey,
                    "selectively invalidating dead route for peer"
                );
                if let Some(blob) = self.route_cache.remove(&pubkey) {
                    self.imported_routes.remove(&blob);
                }
                let _ = api.release_private_route(route_id.clone());
            }
        }
    }
}
