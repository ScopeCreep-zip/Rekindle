//! MeshManager — gossip mesh lifecycle management.
//!
//! Owns the population, refresh, and eviction lifecycle for community
//! gossip meshes. The BroadcastManager owns the mesh data structures
//! and fan-out. The MeshManager owns the policy decisions:
//!
//! - WHEN to populate (join, resume, periodic refresh)
//! - WHICH members to add (read from registry, filter by profile_dht_key presence)
//! - WHEN to evict (TTL-based staleness, member leave events)
//! - HOW to refresh (re-read route blobs from DHT, update timestamps)
//!
//! The MeshManager uses RouteResolver for route resolution. When it
//! discovers a member's route blob, it both populates the gossip mesh
//! (for community broadcast) AND caches the route in PeerRegistry
//! (for direct peer messaging).

use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::broadcast::BroadcastManager;
use crate::broadcast::peer_registry::PeerRegistry;
use crate::resolver::RouteResolver;

const MEMBER_TTL_SECS: u64 = 300;

pub struct MeshManager {
    resolver: Arc<RouteResolver>,
    broadcast: Arc<BroadcastManager>,
    peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
}

impl MeshManager {
    pub fn new(
        resolver: Arc<RouteResolver>,
        broadcast: Arc<BroadcastManager>,
        peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
    ) -> Self {
        Self { resolver, broadcast, peer_registry }
    }

    /// Populate a community mesh from a list of members.
    ///
    /// Called after join completion and during resume. Reads each member's
    /// route blob via the RouteResolver, caches the route, and adds the
    /// member to the gossip mesh.
    ///
    /// `members`: list of (pseudonym_key, profile_dht_key) pairs.
    /// `my_pseudonym`: our own pseudonym in this community (excluded from mesh).
    /// `community_id`: governance key identifying the community.
    pub async fn populate(
        &self,
        community_id: &str,
        my_pseudonym: &str,
        members: &[(String, Option<String>)],
    ) -> u32 {
        let mut discovered = 0u32;

        for (pseudonym, profile_key_opt) in members {
            // Skip self
            if pseudonym == my_pseudonym {
                continue;
            }

            let Some(profile_key) = profile_key_opt.as_deref() else {
                continue;
            };
            if profile_key.is_empty() {
                continue;
            }

            // Register the peer→profile mapping in the resolver
            self.resolver.register_peer(pseudonym, profile_key);

            // Resolve the route (cache hit or DHT read)
            let resolve_result = self.resolver.resolve(pseudonym).await;

            match resolve_result {
                crate::resolver::ResolveResult::Found(_target) => {
                    // Route resolved — read the cached blob to upsert into mesh
                    let peers = &self.peer_registry;
                    let route_blob = {
                        let registry = peers.read();
                        registry.get_route(pseudonym).map(<[u8]>::to_vec)
                    };

                    if let Some(blob) = route_blob {
                        let now_secs = rekindle_utils::timestamp_secs();
                        self.broadcast.upsert_mesh_peer(
                            community_id, pseudonym, blob, "online", now_secs, my_pseudonym,
                        );
                        discovered += 1;
                    }
                }
                crate::resolver::ResolveResult::NoRoute => {
                    debug!(
                        member = &pseudonym[..16.min(pseudonym.len())],
                        "member has no route — skipping mesh upsert"
                    );
                }
                crate::resolver::ResolveResult::UnknownPeer => {
                    // Should not happen — we just registered above
                    warn!(member = &pseudonym[..16.min(pseudonym.len())], "peer unknown after registration — bug");
                }
                crate::resolver::ResolveResult::CircuitOpen => {
                    debug!(
                        member = &pseudonym[..16.min(pseudonym.len())],
                        "circuit open — skipping mesh upsert"
                    );
                }
            }
        }

        info!(
            community = &community_id[..20.min(community_id.len())],
            total = members.len(),
            discovered,
            "mesh populated"
        );

        discovered
    }

    /// Refresh routes for all members currently in a mesh.
    ///
    /// Re-resolves each member's route via RouteResolver (which re-reads
    /// from DHT if the cache is stale). Updates the mesh with fresh
    /// timestamps. Evicts members whose routes can no longer be resolved.
    pub async fn refresh(
        &self,
        community_id: &str,
        my_pseudonym: &str,
    ) {
        let current_members: Vec<String> = {
            let meshes = self.broadcast.meshes();
            let guard = meshes.read();
            match guard.get(community_id) {
                Some(mesh) => mesh.online_members.keys().cloned().collect(),
                None => return,
            }
        };

        let mut refreshed = 0u32;
        let mut evicted = 0u32;

        for pseudonym in &current_members {
            let resolve_result = self.resolver.resolve(pseudonym).await;

            match resolve_result {
                crate::resolver::ResolveResult::Found(_) => {
                    // Route still valid — update timestamp in mesh
                    let peers = &self.peer_registry;
                    let route_blob = {
                        let registry = peers.read();
                        registry.get_route(pseudonym).map(<[u8]>::to_vec)
                    };
                    if let Some(blob) = route_blob {
                        let now_secs = rekindle_utils::timestamp_secs();
                        self.broadcast.upsert_mesh_peer(
                            community_id, pseudonym, blob, "online", now_secs, my_pseudonym,
                        );
                        refreshed += 1;
                    }
                }
                _ => {
                    // Route no longer resolvable — evict from mesh
                    self.broadcast.remove_mesh_peer(community_id, pseudonym);
                    evicted += 1;
                }
            }
        }

        if refreshed > 0 || evicted > 0 {
            debug!(
                community = &community_id[..20.min(community_id.len())],
                refreshed, evicted,
                "mesh refreshed"
            );
        }
    }

    /// Evict stale members from a community mesh based on TTL.
    ///
    /// Members not seen (route not refreshed) within MEMBER_TTL_SECS
    /// are removed from the mesh and their peer set.
    pub fn evict_stale(&self, community_id: &str) {
        let now_secs = rekindle_utils::timestamp_secs();
        let mut meshes = self.broadcast.meshes().write();
        if let Some(mesh) = meshes.get_mut(community_id) {
            let before = mesh.online_members.len();
            mesh.evict_stale(now_secs, MEMBER_TTL_SECS);
            let after = mesh.online_members.len();
            let evicted = before.saturating_sub(after);
            if evicted > 0 {
                info!(
                    community = &community_id[..20.min(community_id.len())],
                    evicted, remaining = after,
                    "stale mesh members evicted"
                );
            }
        }
    }

    /// Evict stale members from ALL community meshes.
    pub fn evict_all_stale(&self) {
        let now_secs = rekindle_utils::timestamp_secs();
        let mut meshes = self.broadcast.meshes().write();
        for (community_id, mesh) in meshes.iter_mut() {
            let before = mesh.online_members.len();
            mesh.evict_stale(now_secs, MEMBER_TTL_SECS);
            let evicted = before.saturating_sub(mesh.online_members.len());
            if evicted > 0 {
                debug!(
                    community = &community_id[..20.min(community_id.len())],
                    evicted,
                    "stale members evicted"
                );
            }
        }
    }

    /// Handle a member joining — register and attempt route resolution.
    pub async fn on_member_joined(
        &self,
        community_id: &str,
        pseudonym: &str,
        profile_dht_key: Option<&str>,
        route_blob: Option<Vec<u8>>,
        my_pseudonym: &str,
    ) {
        // If route_blob provided directly (from gossip payload), use it
        if let Some(blob) = route_blob {
            if !blob.is_empty() {
                // Cache in peer registry
                self.peer_registry.write().cache_route(pseudonym, blob.clone());
                // Add to mesh
                let now_secs = rekindle_utils::timestamp_secs();
                self.broadcast.upsert_mesh_peer(
                    community_id, pseudonym, blob, "online", now_secs, my_pseudonym,
                );
                return;
            }
        }

        // Otherwise, register and resolve via DHT
        if let Some(profile_key) = profile_dht_key {
            if !profile_key.is_empty() {
                self.resolver.register_peer(pseudonym, profile_key);
                if let crate::resolver::ResolveResult::Found(_) = self.resolver.resolve(pseudonym).await {
                    let peers = &self.peer_registry;
                    let blob = peers.read().get_route(pseudonym).map(<[u8]>::to_vec);
                    if let Some(blob) = blob {
                        let now_secs = rekindle_utils::timestamp_secs();
                        self.broadcast.upsert_mesh_peer(
                            community_id, pseudonym, blob, "online", now_secs, my_pseudonym,
                        );
                    }
                }
            }
        }
    }

    /// Handle a member leaving — remove from mesh and invalidate route.
    pub fn on_member_left(&self, community_id: &str, pseudonym: &str) {
        self.broadcast.remove_mesh_peer(community_id, pseudonym);
        self.resolver.invalidate(pseudonym);
        debug!(
            community = &community_id[..20.min(community_id.len())],
            member = &pseudonym[..16.min(pseudonym.len())],
            "member left — removed from mesh"
        );
    }

    /// Handle a route death — invalidate all peers using the dead route
    /// and force re-resolution on next access.
    pub fn on_route_died(&self, _route_id: &str) {
        // Currently we can't map route_id back to peer_key because
        // PeerRegistry caches route blobs, not RouteIds. The imported
        // RouteId is cached inside PeerRegistry.get_or_import but not
        // exposed in a reverse index.
        //
        // For now, the 90s periodic refresh handles route death recovery.
        // A future optimization would add a route_id→peer_key reverse
        // index to PeerRegistry for O(1) invalidation.
        debug!("route died — will recover on next periodic refresh");
    }
}
