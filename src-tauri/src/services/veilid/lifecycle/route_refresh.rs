use std::sync::Arc;

use crate::state::AppState;
use tokio::sync::mpsc;

/// Routeless watchdog — the only timer in the route lifecycle.
///
/// Routes are event-driven: they live until Veilid reports them dead
/// (`handle_route_change` heals immediately) or attachment is lost
/// (`handle_attachment` heals on reconnect). This loop only backstops
/// the cases those events can miss — a failed heal, a startup race —
/// by allocating whenever we're attached with no live route. It never
/// rotates a healthy route (veilid-core manages route health itself;
/// fixed-interval rotation is also a deterministic-timing fingerprint).
pub(crate) async fn route_watchdog_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(rekindle_route::lifecycle::ROUTE_WATCHDOG_INTERVAL);
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // 15-min backstop TTL on cached PEER routes (peers no
                // longer rotate on a timer; dead-remote-route events
                // and send failures are the primary invalidation).
                let evicted = crate::state_helpers::evict_stale_peer_routes(&state);
                if evicted > 0 {
                    tracing::debug!(evicted, "evicted stale peer routes from live cache");
                }
                let missing_route = {
                    let node = state.node.read();
                    node.as_ref()
                        .is_some_and(|nh| nh.is_attached && nh.route_blob.is_none())
                };
                if missing_route {
                    tracing::info!("route watchdog: attached but no live route — allocating");
                    super::super::network::allocate_fresh_private_route(&app_handle, &state).await;
                }
                // Media-class route: same 30s backstop the general route
                // has. Its blob lives on the routing manager (not the node
                // handle), and a missed RouteChange or a failed heal leaves
                // it null — re-allocate whenever attached with no live media
                // route so voice does not fall back to the general route
                // forever after an unobserved media-route death.
                let missing_media_route = {
                    let attached = state.node.read().as_ref().is_some_and(|nh| nh.is_attached);
                    attached
                        && state
                            .routing_manager
                            .read()
                            .as_ref()
                            .is_some_and(|h| h.manager.media_route_blob().is_none())
                };
                if missing_media_route {
                    tracing::info!(
                        "route watchdog: attached but no live media route — allocating"
                    );
                    super::super::media_route::allocate_fresh_media_route(&state).await;
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::debug!("route watchdog loop shutting down");
                break;
            }
        }
    }
}
