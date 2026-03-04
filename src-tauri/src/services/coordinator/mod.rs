//! Coordinator service for the rotating coordinator model.
//!
//! Any eligible online member can be elected coordinator via deterministic
//! hash scoring. The coordinator relays real-time messages, enforces
//! permissions/rate-limits, writes to DHT, and heartbeats every 30s.
//! On failure, members detect the 60s timeout and re-elect.

pub mod audit;
pub mod automod;
pub mod election;
pub mod heartbeat;
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
    /// We are the active coordinator for this community.
    Coordinator,
    /// We are a regular member (someone else is coordinator).
    Member,
}

/// A DHT value change forwarded from the Veilid dispatch loop
/// to the coordinator service for a specific community.
pub struct CoordinatorValueChange {
    pub subkey: u32,
    pub value: Option<Vec<u8>>,
}

/// Lightweight handle stored in AppState for querying/controlling the coordinator service.
pub struct CoordinatorServiceHandle {
    pub community_id: String,
    pub role: Arc<RwLock<CoordinatorRole>>,
    shutdown_tx: mpsc::Sender<()>,
    /// Channel for forwarding VeilidUpdate::ValueChange events from the dispatch loop.
    pub value_change_tx: mpsc::Sender<CoordinatorValueChange>,
    /// State manager for coordinator-side join/moderation/config handling (automod + raid + audit).
    pub state_mgr: Arc<state_manager::StateManager>,
}

impl CoordinatorServiceHandle {
    /// Check if we're currently the coordinator.
    pub fn is_coordinator(&self) -> bool {
        matches!(*self.role.read(), CoordinatorRole::Coordinator)
    }

    /// Signal the coordinator service to shut down.
    pub async fn stop(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }
}

/// Start the coordinator service for a community.
///
/// Spawns election and heartbeat tasks. Returns a handle that can be stored
/// in AppState for querying the role and forwarding value changes.
pub fn start(
    state: Arc<AppState>,
    community_id: String,
) -> CoordinatorServiceHandle {
    let role = Arc::new(RwLock::new(CoordinatorRole::Idle));
    let (shutdown_tx, mut main_shutdown_rx) = mpsc::channel::<()>(1);
    let (value_change_tx, value_change_rx) = mpsc::channel(64);
    let (election_trigger_tx, election_trigger_rx) = mpsc::channel(4);
    let state_mgr = Arc::new(state_manager::StateManager::new(community_id.clone()));

    // Create an election shutdown channel linked to the main shutdown
    let (election_shutdown_tx, election_shutdown_rx) = mpsc::channel(1);
    tokio::spawn(async move {
        let _ = main_shutdown_rx.recv().await;
        let _ = election_shutdown_tx.send(()).await;
    });

    // Spawn election task
    let election_state = state;
    let election_community = community_id.clone();
    let election_role = role.clone();
    let election_state_mgr = state_mgr.clone();
    let election_trigger_tx_clone = election_trigger_tx.clone();
    tokio::spawn(async move {
        election::run(
            election_state,
            election_community,
            election_role,
            election_state_mgr,
            value_change_rx,
            election_trigger_rx,
            election_shutdown_rx,
            election_trigger_tx_clone,
        )
        .await;
    });

    // Run initial election after a brief delay to let Veilid stabilize
    let initial_trigger = election_trigger_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = initial_trigger.send(()).await;
    });

    CoordinatorServiceHandle {
        community_id,
        role,
        shutdown_tx,
        value_change_tx,
        state_mgr,
    }
}
