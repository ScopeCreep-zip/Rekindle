//! Coordinator election logic.
//!
//! Monitors the manifest coordinator subkey for heartbeat timeouts
//! and runs deterministic election rounds when needed.

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use rekindle_protocol::dht::community::{
    election as proto_election, manifest,
    types::{CoordinatorInfo, MANIFEST_AUTOMOD, MANIFEST_COORDINATOR},
};

use crate::state::AppState;
use crate::state_helpers;

use super::{CoordinatorRole, CoordinatorValueChange, state_manager::StateManager};

/// Main election loop.
///
/// Receives value change events for the community's manifest record and
/// election trigger signals (from heartbeat monitor or initial startup).
pub async fn run(
    state: Arc<AppState>,
    community_id: String,
    role: Arc<RwLock<CoordinatorRole>>,
    state_mgr: Arc<StateManager>,
    mut value_change_rx: mpsc::Receiver<CoordinatorValueChange>,
    mut election_trigger_rx: mpsc::Receiver<()>,
    mut shutdown_rx: mpsc::Receiver<()>,
    election_trigger_tx: mpsc::Sender<()>,
) {
    tracing::info!(community = %community_id, "coordinator election loop started");

    // Track active heartbeat/monitor tasks + their shutdown senders.
    // The shutdown sender MUST be kept alive; dropping it causes recv() to return None immediately.
    let mut coordinator_heartbeat_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut coordinator_heartbeat_shutdown: Option<mpsc::Sender<()>> = None;
    let mut member_monitor_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut member_monitor_shutdown: Option<mpsc::Sender<()>> = None;

    loop {
        tokio::select! {
            Some(change) = value_change_rx.recv() => {
                match change.subkey {
                    // Handle coordinator subkey (5) changes
                    MANIFEST_COORDINATOR => {
                        tracing::debug!(community = %community_id, "coordinator info changed in DHT");
                        if let Some(value) = change.value {
                            if let Ok(info) = serde_json::from_slice::<CoordinatorInfo>(&value) {
                                handle_coordinator_change(
                                    &state, &community_id, &role, &info
                                );
                            }
                        } else {
                            // Value not inline -- trigger election to re-read from DHT
                            let _ = election_trigger_tx.try_send(());
                        }
                    }
                    // Handle automod config (subkey 9) changes
                    MANIFEST_AUTOMOD => {
                        tracing::debug!(community = %community_id, "automod config changed in DHT");
                        if let Some(value) = change.value {
                            if let Ok(config) = serde_json::from_slice::<
                                rekindle_protocol::dht::community::automod::AutoModConfig
                            >(&value) {
                                state_mgr.reload_automod(config.clone());
                                state_mgr.reload_raid_config(config.raid_protection);
                                tracing::info!(community = %community_id, "automod config reloaded");
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some(()) = election_trigger_rx.recv() => {
                tracing::info!(community = %community_id, "election triggered");

                match run_election(&state, &community_id).await {
                    Ok(we_won) => {
                        let old_role = *role.read();

                        if we_won {
                            *role.write() = CoordinatorRole::Coordinator;
                            tracing::info!(community = %community_id, "we are now coordinator");

                            // Create audit record if not yet initialized
                            if state_mgr.audit_logger().lock().record_key().is_none() {
                                let audit_state = state.clone();
                                let audit_community = community_id.clone();
                                let audit_logger = state_mgr.audit_logger();
                                tokio::spawn(async move {
                                    if let Err(e) = super::audit::create_audit_record(
                                        &audit_state, &audit_community, &audit_logger,
                                    ).await {
                                        tracing::warn!(
                                            community = %audit_community,
                                            error = %e,
                                            "failed to create audit record"
                                        );
                                    }
                                });
                            }

                            // Stop member monitor if running
                            if let Some(handle) = member_monitor_handle.take() {
                                handle.abort();
                            }
                            member_monitor_shutdown.take();

                            // Start coordinator heartbeat if not already running
                            let should_start = coordinator_heartbeat_handle
                                .as_ref()
                                .is_none_or(tokio::task::JoinHandle::is_finished);
                            if should_start {
                                let hb_state = state.clone();
                                let hb_community = community_id.clone();
                                let hb_state_mgr = state_mgr.clone();
                                let (hb_shutdown_tx, hb_shutdown_rx) = mpsc::channel(1);
                                coordinator_heartbeat_shutdown = Some(hb_shutdown_tx);
                                coordinator_heartbeat_handle = Some(tokio::spawn(async move {
                                    super::heartbeat::run_coordinator_heartbeat(
                                        hb_state,
                                        hb_community,
                                        hb_state_mgr,
                                        hb_shutdown_rx,
                                    ).await;
                                }));
                            }
                        } else {
                            *role.write() = CoordinatorRole::Member;
                            tracing::info!(community = %community_id, "we are a member (not coordinator)");

                            // Stop coordinator heartbeat if we were previously coordinator
                            if old_role == CoordinatorRole::Coordinator {
                                if let Some(handle) = coordinator_heartbeat_handle.take() {
                                    handle.abort();
                                }
                                coordinator_heartbeat_shutdown.take();
                            }

                            // Start member monitor if not already running
                            let should_start = member_monitor_handle
                                .as_ref()
                                .is_none_or(tokio::task::JoinHandle::is_finished);
                            if should_start {
                                let mon_state = state.clone();
                                let mon_community = community_id.clone();
                                let mon_trigger = election_trigger_tx.clone();
                                let (mon_shutdown_tx, mon_shutdown_rx) = mpsc::channel(1);
                                member_monitor_shutdown = Some(mon_shutdown_tx);
                                member_monitor_handle = Some(tokio::spawn(async move {
                                    super::heartbeat::run_member_monitor(
                                        mon_state,
                                        mon_community,
                                        mon_trigger,
                                        mon_shutdown_rx,
                                    ).await;
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(community = %community_id, error = %e, "election failed");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!(community = %community_id, "coordinator election loop shutting down");

                // Clean up heartbeat/monitor tasks (drop senders to signal shutdown, then abort)
                coordinator_heartbeat_shutdown.take();
                member_monitor_shutdown.take();
                if let Some(handle) = coordinator_heartbeat_handle.take() {
                    handle.abort();
                }
                if let Some(handle) = member_monitor_handle.take() {
                    handle.abort();
                }
                break;
            }
        }
    }
}

/// Handle a coordinator info change from DHT.
fn handle_coordinator_change(
    state: &Arc<AppState>,
    community_id: &str,
    role: &Arc<RwLock<CoordinatorRole>>,
    info: &CoordinatorInfo,
) {
    // Update CommunityState with new coordinator info
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        cs.coordinator_pseudonym = Some(info.pseudonym_key.clone());
        cs.coordinator_route_blob = Some(info.route_blob.clone());
        cs.coordinator_epoch = info.epoch;
    }

    // Check if we're the new coordinator
    let my_pseudonym = communities
        .get(community_id)
        .and_then(|cs| cs.my_pseudonym_key.clone());
    drop(communities);

    if let Some(my_key) = my_pseudonym {
        if my_key == info.pseudonym_key {
            *role.write() = CoordinatorRole::Coordinator;
        } else {
            *role.write() = CoordinatorRole::Member;
        }
    }
}

/// Execute one election round.
///
/// Returns `Ok(true)` if we won the election.
pub async fn run_election(state: &Arc<AppState>, community_id: &str) -> Result<bool, String> {
    // Clone routing context BEFORE .await (parking_lot guard is !Send)
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    // Read from CommunityState (clone out of lock)
    let (manifest_key, registry_key, my_pseudonym) = {
        let communities = state.communities.read();
        let c = communities
            .get(community_id)
            .ok_or("community not found")?;
        (
            c.manifest_key
                .clone()
                .or_else(|| Some(c.id.clone()))
                .ok_or("no manifest key")?,
            c.member_registry_key.clone(),
            c.my_pseudonym_key
                .clone()
                .unwrap_or_default(),
        )
    };

    // Open DHT records before reading — they may be closed after app restart.
    // open_record is idempotent: no-op if already open.
    if let Err(e) = mgr.open_record(&manifest_key).await {
        tracing::warn!(community = %community_id, error = %e, "election: failed to open manifest record");
    }
    if let Some(ref reg_key) = registry_key {
        if let Err(e) = mgr.open_record(reg_key).await {
            tracing::warn!(community = %community_id, error = %e, "election: failed to open registry record");
        }
    }

    // Read current coordinator to get epoch
    let current_coordinator = manifest::read_coordinator(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read coordinator: {e}"))?;
    let current_epoch = current_coordinator.as_ref().map_or(0, |c| c.epoch);
    let new_epoch = current_epoch + 1;

    // Read member index
    let members = if let Some(ref reg_key) = registry_key {
        rekindle_protocol::dht::community::member_registry::read_member_index(&mgr, reg_key)
            .await
            .map_err(|e| format!("read member index: {e}"))?
    } else {
        tracing::warn!(community = %community_id, "election: no member_registry_key — member list empty");
        Vec::new()
    };

    // Read roles
    let roles = manifest::read_roles(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read roles: {e}"))?;

    tracing::debug!(
        community = %community_id,
        member_count = members.len(),
        role_count = roles.len(),
        registry_key = ?registry_key,
        "election: read {} members, {} roles",
        members.len(),
        roles.len(),
    );

    let now_secs = rekindle_utils::timestamp_secs();

    // Find the winner
    let winner = proto_election::find_winner(community_id, new_epoch, &members, &roles, now_secs);

    let we_won = winner.as_deref() == Some(&my_pseudonym);

    if we_won {
        // Write ourselves as coordinator
        let route_blob = state_helpers::our_route_blob(state).unwrap_or_default();
        let coordinator_info = CoordinatorInfo {
            pseudonym_key: my_pseudonym.clone(),
            route_blob,
            epoch: new_epoch,
            capabilities: vec![],
            heartbeat_at: now_secs,
        };
        manifest::write_coordinator(&mgr, &manifest_key, &coordinator_info)
            .await
            .map_err(|e| format!("write coordinator: {e}"))?;

        // Update local state
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.coordinator_pseudonym = Some(my_pseudonym);
            cs.coordinator_route_blob = Some(coordinator_info.route_blob);
            cs.coordinator_epoch = new_epoch;
        }

        tracing::info!(
            community = %community_id,
            epoch = new_epoch,
            "won coordinator election"
        );
    } else {
        tracing::info!(
            community = %community_id,
            winner = ?winner,
            "lost coordinator election"
        );
    }

    Ok(we_won)
}
