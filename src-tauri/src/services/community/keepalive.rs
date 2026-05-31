use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use rekindle_protocol::dht::DHTManager;

use crate::state::AppState;
use crate::state_helpers;

// Wave 8 P8.1 — record warming with jitter to defeat timing-analysis
// fingerprinting (`feedback_vulnerable_users_no_creative_paths.md`).
//
// A fixed 300 s cadence is itself a fingerprint: an observer
// correlating Veilid DHT reads can attribute warming events to a
// specific Rekindle peer. The cadence below uses uniform jitter on
// both the inter-warming interval (±20 % of the base) and the per-
// community start offset (0..BASE_INTERVAL) so two clients warming
// the same record never burst in lockstep.
const BASE_INTERVAL: Duration = Duration::from_secs(300);
const INTERVAL_JITTER_SECS: i64 = 60; // ± 60 s on top of BASE_INTERVAL

fn next_warming_delay() -> Duration {
    // Sample a signed offset in `±INTERVAL_JITTER_SECS` and apply it
    // to `BASE_INTERVAL`. `saturating_add_signed` keeps the result
    // non-negative even when offset is at the negative extreme.
    let base_secs = BASE_INTERVAL.as_secs();
    let offset_signed: i64 =
        rand::thread_rng().gen_range(-INTERVAL_JITTER_SECS..=INTERVAL_JITTER_SECS);
    let secs = base_secs.saturating_add_signed(offset_signed);
    Duration::from_secs(secs)
}

fn initial_offset() -> Duration {
    // Stagger per-community start so a fresh login that joined N
    // communities doesn't warm them all at exactly the same wall-
    // clock instant. Range is [0, BASE_INTERVAL) so the first warming
    // for any community lands somewhere within the first base window.
    Duration::from_secs(rand::thread_rng().gen_range(0..BASE_INTERVAL.as_secs()))
}

/// Start a DHT keepalive task that re-accesses community DHT records
/// at a jittered ~5 minute cadence to prevent them from expiring in
/// the Veilid DHT (architecture §14.1 mutual aid: usage IS maintenance).
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
        // Stagger startup so simultaneous logins across N communities
        // don't trigger a synchronized warming burst.
        let first_sleep = initial_offset();
        tokio::select! {
            () = tokio::time::sleep(first_sleep) => {}
            _ = shutdown_rx.recv() => return,
        }

        loop {
            let Some(rc) = state_helpers::safe_routing_context(&state) else {
                tokio::select! {
                    () = tokio::time::sleep(next_warming_delay()) => continue,
                    _ = shutdown_rx.recv() => return,
                }
            };
            let keys = {
                let communities = state.communities.read();
                communities.get(&community_id).map(|c| {
                    let mut keys = Vec::new();
                    if let Some(key) = c.governance_key.clone() {
                        keys.push(key);
                    } else {
                        keys.push(c.id.clone());
                    }
                    if let Some(key) = c.member_registry_key.clone() {
                        keys.push(key);
                    }
                    keys.extend(c.channel_log_keys.values().cloned());
                    // Mutual Aid (architecture §14.1) + Plate Gate
                    // (§15.4): warm segment-N governance, registry,
                    // and channel-segment records too — otherwise
                    // expansion segments expire while the rest stay
                    // hot, fragmenting the community's DHT presence.
                    if let Some(gov) = c.governance_state.as_ref() {
                        for seg in &gov.segments {
                            keys.push(seg.governance_key.clone());
                            keys.push(seg.registry_key.clone());
                        }
                        for csr in gov.channel_segment_records.values() {
                            keys.push(csr.record_key.clone());
                        }
                    }
                    keys
                })
            };
            if let Some(keys) = keys {
                // Touch the live v2 records by reading subkey 0 to
                // prevent DHT expiry. Do NOT call open_record — it
                // clobbers the writer on re-open, which would
                // downgrade a writable open to read-only.
                let mgr = DHTManager::new(rc);
                for key in keys {
                    let _ = mgr.get_value(&key, 0).await;
                }
            }

            // Sleep with per-cycle jitter so the cadence pattern is
            // not a fingerprint.
            tokio::select! {
                () = tokio::time::sleep(next_warming_delay()) => {}
                _ = shutdown_rx.recv() => return,
            }
        }
    });
}
