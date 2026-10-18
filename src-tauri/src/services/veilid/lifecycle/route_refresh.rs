use std::sync::Arc;

use crate::state::AppState;
use tokio_util::sync::CancellationToken;

/// Peer-route eviction (→ E2.1): a 15-minute backstop TTL on the peer routes
/// we cached. Peers no longer rotate on a timer; dead-remote-route events and
/// send failures are the primary invalidation.
///
/// Our own routes are not touched here: `OwnRoutes` (plan C7.9a) reallocates
/// a dead route at once and retries a `TryAgain` with backoff, and the
/// transport's route republisher backstops an allocation that failed for
/// good (plan E1.1). Nothing rotates a healthy route (veilid-core manages
/// route health; fixed-interval rotation is a deterministic-timing
/// fingerprint).
pub(crate) async fn route_watchdog_loop(state: Arc<AppState>, stop: CancellationToken) {
    let mut interval = tokio::time::interval(rekindle_route::lifecycle::ROUTE_WATCHDOG_INTERVAL);
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let evicted = crate::state_helpers::evict_stale_peer_routes(&state);
                if evicted > 0 {
                    tracing::debug!(evicted, "evicted stale peer routes from live cache");
                }
            }
            () = stop.cancelled() => {
                tracing::debug!("peer route eviction shutting down");
                break;
            }
        }
    }
}
