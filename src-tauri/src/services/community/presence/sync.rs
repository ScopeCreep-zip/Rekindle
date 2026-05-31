//! Phase 21 REDO — thin facade.
//!
//! `run_initial_sync` body lives in
//! `rekindle_presence::community::sync::run_initial_sync`
//! parameterised over `CommunityPresenceDeps`. This wrapper
//! constructs a `PresenceAdapter` per call and delegates.
//!
//! Exposed at `services::community::run_initial_sync` for src-tauri
//! callers that need to fire one initial-sync round without waiting
//! for the next presence-poll cadence tick — re-join flows after
//! `leave_community`, admin "force resync" commands, recovery
//! sweeps. The crate's `presence_poll_tick` calls
//! `rekindle_presence::run_initial_sync` directly when its
//! `needs_initial_sync` gate fires; this facade is the explicit
//! on-demand entry point.

use std::sync::Arc;

use crate::state::AppState;

pub async fn run_initial_sync(state: &Arc<AppState>, community_id: &str, d: usize) {
    let Some(adapter) = crate::services::presence_adapter::build_adapter(state) else {
        tracing::debug!(
            community = %community_id,
            "run_initial_sync: adapter unavailable",
        );
        return;
    };
    let adapter = Arc::new(adapter);
    rekindle_presence::run_initial_sync(adapter, community_id, d).await;
}
