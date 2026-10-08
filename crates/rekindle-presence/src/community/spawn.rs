//! Phase 21 REDO — presence-poll outer cadence loop.
//!
//! Pre-port lived in `src-tauri/services/community/presence/poll.rs`
//! as `start_presence_poll`. Owns the architecture §13.4 timing:
//! one initial fire-once tick → six rapid 5-second ticks → 60-second
//! steady-state cadence — stopped by the community's session scope.

use std::sync::Arc;
use std::time::Duration;

use rekindle_lifecycle::SessionScope;
use tokio::time::interval;

use crate::deps::CommunityPresenceDeps;

/// Number of rapid (5-second) ticks fired after the initial tick
/// before settling into the 60-second steady-state cadence.
pub const RAPID_TICKS: usize = 6;
/// Per-tick cadence during the rapid bring-up phase.
pub const RAPID_TICK_INTERVAL_SECS: u64 = 5;
/// Steady-state cadence after the rapid bring-up.
pub const STEADY_TICK_INTERVAL_SECS: u64 = 60;

/// Spawn the presence-poll loop for a community into `scope`, the
/// community's session scope; shutting that scope down (leaving the
/// community, logging out, locking) stops the loop.
///
/// 1. Runs one immediate tick (used to surface a fast first
///    presence write after `join_community` returns).
/// 2. Runs six rapid 5 s ticks so newcomers learn the existing
///    member set without waiting a full minute.
/// 3. Settles into 60 s ticks until the scope shuts down.
///
/// The loop stops at its next safe point when the scope closes: between
/// ticks, or inside a tick before its next Veilid call (plan C4.L1).
pub fn start_presence_poll<D: CommunityPresenceDeps>(
    deps: Arc<D>,
    community_id: String,
    scope: &Arc<SessionScope>,
) {
    scope.spawn_with_token_or_drop("community presence poll", |stop| async move {
        if let Err(error) = deps.run_presence_poll_tick(&community_id).await {
            tracing::warn!(
                community = %community_id,
                %error,
                "initial presence poll tick failed",
            );
        }

        // Rapid bring-up: six 5-second ticks so a fresh joiner
        // learns the existing member set quickly.
        let mut rapid = interval(Duration::from_secs(RAPID_TICK_INTERVAL_SECS));
        rapid.tick().await; // skip immediate fire
        for tick_num in 0..RAPID_TICKS {
            if stop.run_until_cancelled(rapid.tick()).await.is_none() {
                return;
            }
            if let Err(error) = deps.run_presence_poll_tick(&community_id).await {
                tracing::trace!(
                    community = %community_id,
                    tick = tick_num + 1,
                    %error,
                    "rapid presence poll tick failed",
                );
            }
        }

        // Steady state: 60-second cadence until the scope shuts down.
        let mut steady = interval(Duration::from_secs(STEADY_TICK_INTERVAL_SECS));
        steady.tick().await; // skip immediate fire
        loop {
            if stop.run_until_cancelled(steady.tick()).await.is_none() {
                return;
            }
            if let Err(error) = deps.run_presence_poll_tick(&community_id).await {
                tracing::debug!(
                    community = %community_id,
                    %error,
                    "presence poll tick failed",
                );
            }
        }
    });
}
