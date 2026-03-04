//! Coordinator heartbeat and member-side heartbeat monitor.
//!
//! - Coordinator: writes `CoordinatorInfo` to manifest every 30 seconds.
//! - Member: monitors heartbeat_at; triggers re-election on 60s timeout.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use rekindle_protocol::dht::community::{manifest, types::CoordinatorInfo};

use crate::state::AppState;
use crate::state_helpers;

/// Coordinator heartbeat interval.
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Member-side timeout: if no heartbeat for this many seconds, trigger election.
const HEARTBEAT_TIMEOUT_SECS: u64 = 60;

/// Coordinator heartbeat: write CoordinatorInfo every 30 seconds.
///
/// Runs only when we are the active coordinator for a community.
/// Also checks for raid auto-resolve on each tick.
pub async fn run_coordinator_heartbeat(
    state: Arc<AppState>,
    community_id: String,
    state_mgr: Arc<super::state_manager::StateManager>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!(community = %community_id, "coordinator heartbeat started");

    let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    // Skip the first tick (fires immediately)
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = write_heartbeat(&state, &community_id).await {
                    tracing::warn!(
                        community = %community_id,
                        error = %e,
                        "failed to write coordinator heartbeat"
                    );
                }

                // Check raid auto-resolve
                let now = rekindle_utils::timestamp_secs();
                if state_mgr.check_raid_auto_resolve(now) {
                    tracing::info!(
                        community = %community_id,
                        "raid protection auto-resolved"
                    );
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!(community = %community_id, "coordinator heartbeat shutting down");
                break;
            }
        }
    }
}

/// Write a heartbeat immediately (called after route refresh to prevent stale routes).
pub async fn write_heartbeat_now(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    write_heartbeat(state, community_id).await
}

/// Write a heartbeat to the manifest coordinator subkey.
async fn write_heartbeat(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    // Clone routing context and community data BEFORE .await (parking_lot guard is !Send)
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let (manifest_key, my_pseudonym, epoch) = {
        let communities = state.communities.read();
        let c = communities
            .get(community_id)
            .ok_or("community not found")?;
        (
            c.manifest_key
                .clone()
                .or_else(|| Some(c.id.clone()))
                .ok_or("no manifest key")?,
            c.my_pseudonym_key.clone().unwrap_or_default(),
            c.coordinator_epoch,
        )
    };

    let route_blob = state_helpers::our_route_blob(state).unwrap_or_default();
    let now_secs = rekindle_utils::timestamp_secs();

    let coordinator_info = CoordinatorInfo {
        pseudonym_key: my_pseudonym,
        route_blob,
        epoch,
        capabilities: vec![],
        heartbeat_at: now_secs,
    };

    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    manifest::write_coordinator(&mgr, &manifest_key, &coordinator_info)
        .await
        .map_err(|e| format!("write coordinator: {e}"))?;

    tracing::debug!(
        community = %community_id,
        epoch,
        heartbeat_at = now_secs,
        "wrote coordinator heartbeat"
    );

    Ok(())
}

/// Member-side heartbeat monitor: trigger election on 60s timeout.
///
/// Periodically checks the coordinator's `heartbeat_at` from cached state.
/// If it's older than `HEARTBEAT_TIMEOUT_SECS`, sends a trigger to the
/// election loop.
pub async fn run_member_monitor(
    state: Arc<AppState>,
    community_id: String,
    election_trigger_tx: mpsc::Sender<()>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!(community = %community_id, "member heartbeat monitor started");

    let check_interval = Duration::from_secs(15);
    let mut interval = tokio::time::interval(check_interval);
    // Skip the first tick
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Read coordinator info from DHT to check heartbeat freshness
                let Some(rc) = state_helpers::routing_context(&state) else {
                    continue;
                };

                let manifest_key = {
                    let communities = state.communities.read();
                    communities.get(&community_id)
                        .and_then(|c| c.manifest_key.clone().or_else(|| Some(c.id.clone())))
                };

                let Some(manifest_key) = manifest_key else {
                    continue;
                };

                let mgr = rekindle_protocol::dht::DHTManager::new(rc);
                match manifest::read_coordinator(&mgr, &manifest_key).await {
                    Ok(Some(info)) => {
                        let now = rekindle_utils::timestamp_secs();
                        let age = now.saturating_sub(info.heartbeat_at);

                        if age > HEARTBEAT_TIMEOUT_SECS {
                            tracing::warn!(
                                community = %community_id,
                                age_secs = age,
                                "coordinator heartbeat timed out — triggering election"
                            );
                            let _ = election_trigger_tx.send(()).await;
                        }
                    }
                    Ok(None) => {
                        // No coordinator set — trigger election
                        tracing::info!(
                            community = %community_id,
                            "no coordinator set — triggering election"
                        );
                        let _ = election_trigger_tx.send(()).await;
                    }
                    Err(e) => {
                        tracing::debug!(
                            community = %community_id,
                            error = %e,
                            "failed to read coordinator info for heartbeat check"
                        );
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!(community = %community_id, "member heartbeat monitor shutting down");
                break;
            }
        }
    }
}
