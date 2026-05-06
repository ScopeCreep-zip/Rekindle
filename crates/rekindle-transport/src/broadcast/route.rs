//! Route lifecycle — allocate, import, release, refresh, publish.
//!
//! Consolidates all route operations. No other code calls Veilid route
//! primitives directly.

use tracing::{debug, info, warn};

use crate::error::Result;
use super::node::TransportNode;
use super::peer_registry::PeerTarget;

// ── Allocation ─────────────────────────────────────────────────────────

/// Allocate a personal private route with default deadline (30 min).
pub async fn allocate_personal(node: &TransportNode) -> Result<(String, Vec<u8>)> {
    debug!("route: allocating personal");
    let result = node.allocate_route().await;
    match &result {
        Ok((id, blob)) => info!(route_id = %id, blob_bytes = blob.len(), "route: personal allocated"),
        Err(e) => warn!(error = %e, "route: personal allocation failed"),
    }
    result
}

/// Allocate a personal private route with a custom deadline in seconds.
pub async fn allocate_with_deadline(
    node: &TransportNode, max_wait_secs: u64,
) -> Result<(String, Vec<u8>)> {
    debug!(max_wait_secs, "route: allocating with deadline");
    let result = node.allocate_route_with_deadline(max_wait_secs).await;
    match &result {
        Ok((id, _)) => info!(route_id = %id, max_wait_secs, "route: allocated with deadline"),
        Err(e) => warn!(error = %e, max_wait_secs, "route: deadline allocation failed"),
    }
    result
}

/// Allocate a community-specific route.
pub async fn allocate_community(node: &TransportNode) -> Result<(String, Vec<u8>)> {
    debug!("route: allocating community");
    let result = node.allocate_route().await;
    match &result {
        Ok((id, _)) => info!(route_id = %id, "route: community allocated"),
        Err(e) => warn!(error = %e, "route: community allocation failed"),
    }
    result
}

/// Allocate a voice-specific route.
pub async fn allocate_voice(node: &TransportNode) -> Result<(String, Vec<u8>)> {
    debug!("route: allocating voice");
    let result = node.allocate_route().await;
    match &result {
        Ok((id, _)) => info!(route_id = %id, "route: voice allocated"),
        Err(e) => warn!(error = %e, "route: voice allocation failed"),
    }
    result
}

// ── Import ─────────────────────────────────────────────────────────────

/// Import a remote peer's route blob, returning a `PeerTarget` handle.
pub fn import_peer_route(node: &TransportNode, route_blob: &[u8]) -> Result<PeerTarget> {
    debug!(blob_bytes = route_blob.len(), "route: importing peer");
    let result = node.import_route(route_blob);
    if let Err(ref e) = result {
        warn!(error = %e, "route: peer import failed");
    }
    result
}

// ── Release ────────────────────────────────────────────────────────────

/// Forget the current personal route (used when Veilid reports it dead).
pub fn forget_personal_route(node: &TransportNode) {
    info!("route: forgetting personal (dead)");
    node.routes().write().forget_route();
}

// ── Publish ────────────────────────────────────────────────────────────

/// Publish a route blob to the personal profile DHT record.
pub async fn publish_to_profile(
    node: &TransportNode, profile_key: &str, route_blob: &[u8],
) -> Result<()> {
    debug!(profile_key, blob_bytes = route_blob.len(), "route: publishing to profile");
    super::dht_writes::set(
        node, profile_key,
        crate::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
        route_blob.to_vec(), None,
    ).await
}

/// Publish a route blob to the personal mailbox DHT record.
pub async fn publish_to_mailbox(
    node: &TransportNode, mailbox_key: &str, route_blob: &[u8],
) -> Result<()> {
    debug!(mailbox_key, blob_bytes = route_blob.len(), "route: publishing to mailbox");
    node.dht()?.mailbox().update_route(mailbox_key, route_blob).await
}

/// Publish a community route blob to the community mailbox.
pub async fn publish_to_community_mailbox(
    node: &TransportNode, community_mailbox_key: &str, route_blob: &[u8],
) -> Result<()> {
    debug!(community_mailbox_key, blob_bytes = route_blob.len(), "route: publishing to community mailbox");
    node.dht()?.mailbox().update_community_route(community_mailbox_key, route_blob).await
}

// ── Refresh ────────────────────────────────────────────────────────────

/// Refresh the personal route: forget old, allocate new, publish to profile + mailbox.
pub async fn refresh_personal(
    node: &TransportNode, profile_key: &str, mailbox_key: &str,
) -> Result<Vec<u8>> {
    info!("route: refreshing personal");
    forget_personal_route(node);
    let (_route_id, route_blob) = node.allocate_route().await?;
    let _ = publish_to_profile(node, profile_key, &route_blob).await;
    let _ = publish_to_mailbox(node, mailbox_key, &route_blob).await;
    info!("route: personal refreshed");
    Ok(route_blob)
}

/// Refresh a community route: allocate new, publish to community mailbox.
pub async fn refresh_community(
    node: &TransportNode, community_mailbox_key: &str,
) -> Result<Vec<u8>> {
    debug!(community_mailbox_key, "route: refreshing community");
    let (_route_id, route_blob) = node.allocate_route().await?;
    publish_to_community_mailbox(node, community_mailbox_key, &route_blob).await?;
    info!(community_mailbox_key, "route: community refreshed");
    Ok(route_blob)
}

/// Refresh all routes: personal + all operator community routes.
pub async fn refresh_all(
    node: &TransportNode, profile_key: &str, mailbox_key: &str,
    operator_communities: &[(String, String)],
) -> usize {
    info!(communities = operator_communities.len(), "route: refreshing all");
    let mut refreshed = 0;

    match refresh_personal(node, profile_key, mailbox_key).await {
        Ok(_) => { refreshed += 1; }
        Err(e) => warn!(error = %e, "route: personal refresh failed"),
    }

    for (name, mailbox) in operator_communities {
        match refresh_community(node, mailbox).await {
            Ok(_) => {
                refreshed += 1;
                debug!(community = %name, "route: community refreshed");
            }
            Err(e) => warn!(community = %name, error = %e, "route: community refresh failed"),
        }
    }

    info!(refreshed, total = 1 + operator_communities.len(), "route: refresh_all complete");
    refreshed
}
