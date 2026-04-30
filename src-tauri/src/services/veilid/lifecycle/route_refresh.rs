use std::sync::Arc;
use std::time::Instant;

use crate::state::AppState;
use tokio::sync::mpsc;

/// Periodically re-allocate our private route to prevent silent expiration.
pub(crate) async fn route_refresh_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(rekindle_route::lifecycle::ROUTE_REFRESH_INTERVAL);
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = Instant::now();
                let evicted = crate::state_helpers::evict_stale_peer_routes(&state);
                if evicted > 0 {
                    tracing::debug!(evicted, "evicted stale peer routes from live cache");
                }
                let should_refresh = {
                    let node = state.node.read();
                    let has_live_route =
                        node.as_ref().is_some_and(|nh| nh.is_attached && nh.route_blob.is_some());
                    drop(node);

                    let routing_manager = state.routing_manager.read();
                    let lifecycle_ready = routing_manager
                        .as_ref()
                        .is_some_and(|handle| handle.route_lifecycle.should_refresh_at(now));
                    has_live_route && lifecycle_ready
                };
                if should_refresh {
                    tracing::debug!("proactive route refresh: re-allocating private route");
                    super::super::network::reallocate_private_route(&app_handle, &state).await;

                    let all_community_ids: Vec<String> = {
                        let communities = state.communities.read();
                        communities.keys().cloned().collect()
                    };

                    {
                        let mut communities = state.communities.write();
                        for community_id in &all_community_ids {
                            if let Some(cs) = communities.get_mut(community_id) {
                                if let Some(ref mut gossip) = cs.gossip {
                                    gossip.needs_initial_sync = true;
                                }
                            }
                        }
                    }

                    for community_id in &all_community_ids {
                        let _ = crate::services::community::rejoin_community(&state, community_id).await;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::debug!("route refresh loop shutting down");
                break;
            }
        }
    }
}
