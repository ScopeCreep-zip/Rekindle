//! Phase 21 REDO — thin facade.
//!
//! `presence_poll_tick` + `start_presence_poll` cadence loop both
//! live in `rekindle_presence::community::{poll, spawn}`. This module
//! constructs a `PresenceAdapter` per call and delegates.
//!
//! Public surface preserved:
//! - `start_presence_poll(state, community_id)` — spawn the cadence loop
//! - `presence_poll_tick_public(state, community_id)` — run one tick
//!   (used by commands::community::presence + sync_service paths)

use std::sync::Arc;

use crate::state::AppState;

/// Start the presence-poll loop for a community. Delegates to
/// `rekindle_presence::start_presence_poll` which owns the cadence
/// (1 initial tick → 6 rapid 5 s → 60 s steady-state).
pub fn start_presence_poll(state: &Arc<AppState>, community_id: String) {
    let Some(adapter) = crate::services::presence_adapter::build_adapter(state) else {
        tracing::warn!(
            community = %community_id,
            "start_presence_poll: adapter unavailable",
        );
        return;
    };
    let scope = crate::state_helpers::community_scope(state, &community_id);
    rekindle_presence::start_presence_poll(Arc::new(adapter), community_id, &scope);
}

/// Run one presence-poll tick on demand. Public entry point used by
/// commands + sync paths that want to bring a community's overlay
/// up-to-date without waiting for the next cadence tick.
pub async fn presence_poll_tick_public(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(), String> {
    let Some(adapter) = crate::services::presence_adapter::build_adapter(state) else {
        return Err("adapter unavailable".to_string());
    };
    let stop = crate::state_helpers::community_scope(state, community_id).token();
    // Leaving the community stops its scope, which the pool's drain does not
    // cover (that is logout's, C7.6g). The tick's DHT work is record-pool
    // calls only (its sends are queued to the gossip sender), and the pool
    // runs each call on its own scope, so leaving it drops none (C4.L1).
    stop.run_until_cancelled(rekindle_presence::presence_poll_tick(
        Arc::new(adapter),
        community_id,
    ))
    .await
    .unwrap_or(Ok(()))
}

/// Kick a single presence-poll tick for a community NOW, in the
/// background — don't wait for the next cadence tick.
///
/// The poll loop does a 6-tick rapid burst only at community login,
/// then settles to a 60 s steady cadence. Any membership-change moment
/// that happens in steady state therefore pays up to a 60 s discovery
/// lag unless it nudges: a voice join won't see who is already in the
/// channel (the reconcile runs on the poll tick), and a profile update
/// won't re-publish its row promptly. Both funnel through here so the
/// "refresh presence now" behaviour lives in one place, on the backend
/// every frontend drives — not inlined per caller.
///
/// Fire-and-forget by design: the caller never blocks on the DHT scan,
/// and a failure is best-effort (debug-logged). Backend-only — the thin
/// frontends just issue the join/update; the discovery nudge is ours.
pub fn nudge_presence_poll(state: &Arc<AppState>, community_id: &str) {
    let state = Arc::clone(state);
    let community_id = community_id.to_string();
    crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop(
        "presence poll nudge",
        async move {
            if let Err(e) = presence_poll_tick_public(&state, &community_id).await {
                tracing::debug!(
                    community = %community_id,
                    error = %e,
                    "presence poll nudge skipped",
                );
            }
        },
    );
}
