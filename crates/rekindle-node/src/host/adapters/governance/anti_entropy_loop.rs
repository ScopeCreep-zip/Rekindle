//! The governance anti-entropy loop (plan E3.2 S7, `evidence/slot-integrity.md`
//! §4.3 (iii)), one task per unlock on both modes: every
//! [`SWEEP_PERIOD`](rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD)
//! each community's sweep inspects 16 random member records, and the first
//! tick after the route comes back (a reconnect) sweeps all of them; the
//! login stage runs the login sweep. A sweep that stored a head rebuilds
//! that community, and each tick moves the community's watch ring, which
//! opens and watches a ring record an earlier move could not, and each
//! tick may propose or endorse a checkpoint hint (S7c.4). The ticks are tokio time (paused under netsim).

use std::collections::BTreeMap;

use rekindle_governance_runtime::anti_entropy::{self, Sweep, SWEEP_PERIOD};
use rekindle_governance_runtime::checkpointing::{self, CheckpointClock, CheckpointStep};
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

pub(super) async fn run(host: HostGovernance, stop: CancellationToken) {
    let mut ticks =
        tokio::time::interval_at(tokio::time::Instant::now() + SWEEP_PERIOD, SWEEP_PERIOD);
    let mut clocks: BTreeMap<String, CheckpointClock> = BTreeMap::new();
    let mut offline = false;
    while stop.run_until_cancelled(ticks.tick()).await.is_some() {
        if host.our_route_blob_impl().is_empty() {
            offline = true;
            continue;
        }
        let kind = if std::mem::take(&mut offline) {
            Sweep::Full
        } else {
            Sweep::Sample
        };
        for community_id in host.sync_targets() {
            if stop.is_cancelled() {
                return;
            }
            // A ring record that failed to open is opened again (the pool
            // re-arms a refused watch itself).
            host.rearm_watch_ring(&community_id).await;
            // The frontier reads the ring and hints share (review S7m part 2).
            let frontiers = host.seen_frontiers(&community_id);
            match anti_entropy::sweep(&host, &community_id, kind, &frontiers).await {
                Ok(0) => {}
                Ok(stored) => {
                    tracing::debug!(community = %community_id, stored, "anti-entropy stored heads");
                    rekindle_governance_runtime::rebuild_governance(&host, &community_id).await;
                }
                Err(error) => {
                    tracing::debug!(community = %community_id, %error, "anti-entropy sweep failed");
                }
            }
            // Propose or endorse a checkpoint hint (plan E3.2 S7c.4): both
            // triggers are local and run on this tick's clock.
            let clock = clocks.entry(community_id.clone()).or_default();
            match checkpointing::tick(&host, &community_id, clock).await {
                Ok(CheckpointStep::Idle) => {}
                Ok(step) => tracing::debug!(community = %community_id, ?step, "checkpoint"),
                Err(error) => {
                    tracing::debug!(community = %community_id, %error, "checkpoint tick failed");
                }
            }
        }
    }
}
