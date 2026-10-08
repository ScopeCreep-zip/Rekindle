use std::sync::Arc;

use crate::state::AppState;
use rekindle_protocol::own_routes::RouteClass;
use tokio_util::sync::CancellationToken;

/// Route watchdog: the only timer in the route lifecycle.
///
/// Our own routes are owned by `OwnRoutes` (plan C7.9a): a death reported
/// in `RouteChange` reallocates at once, and a `TryAgain` is retried with
/// backoff. This loop backstops only what those miss, a route whose
/// allocation failed for good, by wanting both classes again, which is a
/// no-op while a route is live or allocating. It never rotates a healthy
/// route (veilid-core manages route health itself; fixed-interval rotation
/// is also a deterministic-timing fingerprint). It also evicts stale peer
/// routes.
pub(crate) async fn route_watchdog_loop(state: Arc<AppState>, stop: CancellationToken) {
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
                if let Some(routes) = crate::state_helpers::own_routes(&state) {
                    routes.want(RouteClass::General);
                    routes.want(RouteClass::Media);
                }
            }
            () = stop.cancelled() => {
                tracing::debug!("route watchdog loop shutting down");
                break;
            }
        }
    }
}
