//! Coordinator service for the static owner-as-coordinator model.
//!
//! The community creator permanently owns the manifest keypair and acts as
//! coordinator for Tier 2 operations (state changes, moderation, MEK rotation).
//! Tier 1 operations (chat, typing, reactions, presence) use the gossip mesh
//! and do not require a coordinator.
//!
//! No election or heartbeat — the creator is always the coordinator.

pub mod audit;
pub mod automod;
pub mod onboarding;
pub mod raid;
pub mod state_manager;
pub mod timeout;

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::state::AppState;

/// Current role of this node in a community.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorRole {
    /// Not participating in coordinator duties.
    Idle,
    /// We are the active coordinator for this community (we hold the manifest keypair).
    Coordinator,
    /// We are a regular member (the creator/admin is the coordinator).
    Member,
}

/// A DHT value change forwarded from the Veilid dispatch loop.
/// Currently only used to detect manifest changes (automod config, etc.).
pub struct CoordinatorValueChange {
    pub subkey: u32,
    pub value: Option<Vec<u8>>,
}

/// Lightweight handle stored in AppState for querying/controlling the coordinator service.
pub struct CoordinatorServiceHandle {
    pub community_id: String,
    pub role: Arc<RwLock<CoordinatorRole>>,
    /// Channel for forwarding VeilidUpdate::ValueChange events.
    /// The receiver is consumed by the value change monitoring task.
    pub value_change_tx: mpsc::Sender<CoordinatorValueChange>,
    /// State manager for coordinator-side join/moderation/config handling (automod + raid + audit).
    pub state_mgr: Arc<state_manager::StateManager>,
}

impl CoordinatorServiceHandle {
    /// Check if we're currently the coordinator.
    pub fn is_coordinator(&self) -> bool {
        matches!(*self.role.read(), CoordinatorRole::Coordinator)
    }
}

/// Create the coordinator service handle for a community.
///
/// This creates the StateManager (automod, raid, audit) and a value change
/// channel for manifest DHT updates. No election or heartbeat tasks are spawned —
/// the creator is the permanent coordinator (static owner model).
pub fn create_handle(
    state: &Arc<AppState>,
    community_id: String,
) -> CoordinatorServiceHandle {
    let role = Arc::new(RwLock::new(CoordinatorRole::Idle));
    let (value_change_tx, mut value_change_rx) = mpsc::channel::<CoordinatorValueChange>(64);
    let state_mgr = Arc::new(state_manager::StateManager::new(community_id.clone()));

    // Spawn a lightweight task that monitors manifest value changes
    // for automod/raid config reloads (replaces the election loop's
    // value change handling without the election logic).
    let monitor_state = Arc::clone(state);
    let monitor_community = community_id.clone();
    let monitor_state_mgr = state_mgr.clone();
    tokio::spawn(async move {
        use rekindle_protocol::dht::community::manifest;

        while let Some(change) = value_change_rx.recv().await {
            // Manifest subkey 9 = AutoModConfig — reload on change
            if change.subkey == 9 {
                let rc = crate::state_helpers::routing_context(&monitor_state);
                if let Some(rc) = rc {
                    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
                    let manifest_key = {
                        let communities = monitor_state.communities.read();
                        communities
                            .get(&monitor_community)
                            .and_then(|cs| cs.manifest_key.clone())
                    };
                    if let Some(ref mk) = manifest_key {
                        if let Ok(Some(config)) = manifest::read_automod(&mgr, mk).await {
                            monitor_state_mgr.reload_raid_config(config.raid_protection.clone());
                            monitor_state_mgr.reload_automod(config);
                        }
                    }
                }
            }
        }
    });

    CoordinatorServiceHandle {
        community_id,
        role,
        value_change_tx,
        state_mgr,
    }
}
