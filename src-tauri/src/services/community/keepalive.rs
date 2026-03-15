use std::sync::Arc;

use rekindle_protocol::dht::DHTManager;

use crate::state::AppState;
use crate::state_helpers;

/// Start a DHT keepalive task that re-accesses community DHT records every 5 minutes
/// to prevent them from expiring in the Veilid DHT.
pub fn start_dht_keepalive(state: Arc<AppState>, community_id: String) {
    use tokio::sync::mpsc;

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            cs.dht_keepalive_shutdown_tx = Some(shutdown_tx);
        }
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await; // skip immediate first tick
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let Some(rc) = state_helpers::routing_context(&state) else {
                        continue;
                    };
                    let manifest_key = {
                        let communities = state.communities.read();
                        communities
                            .get(&community_id)
                            .and_then(|c| c.manifest_key.clone().or_else(|| Some(c.id.clone())))
                    };
                    let Some(key) = manifest_key else { continue };
                    // Touch the record by reading subkey 0 to prevent DHT expiry.
                    // Do NOT call open_record — it clobbers the writer on re-open,
                    // which would downgrade a writable open to read-only.
                    let mgr = DHTManager::new(rc);
                    let _ = mgr.get_value(&key, 0).await;
                }
                _ = shutdown_rx.recv() => break,
            }
        }
    });
}
