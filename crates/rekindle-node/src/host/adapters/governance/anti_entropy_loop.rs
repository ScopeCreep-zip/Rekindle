//! The governance anti-entropy loop (plan E3.2 S7, `evidence/slot-integrity.md`
//! §4.3 (iii)), one task per unlock on both modes: every
//! [`SWEEP_PERIOD`](rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD)
//! each community's sweep inspects 16 random member records, and the first
//! tick after the route comes back (a reconnect) sweeps all of them; the
//! login stage runs the login sweep. A sweep that stored a head rebuilds
//! that community. The ticks are tokio time (paused under netsim).

use std::collections::BTreeMap;

use rekindle_governance_runtime::anti_entropy::{self, SeenFrontiers, Sweep, SWEEP_PERIOD};
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

pub(super) async fn run(host: HostGovernance, stop: CancellationToken) {
    let mut ticks =
        tokio::time::interval_at(tokio::time::Instant::now() + SWEEP_PERIOD, SWEEP_PERIOD);
    let mut seen: BTreeMap<String, SeenFrontiers> = BTreeMap::new();
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
            let frontiers = seen.entry(community_id.clone()).or_default();
            match anti_entropy::sweep(&host, &community_id, kind, frontiers).await {
                Ok(0) => {}
                Ok(stored) => {
                    tracing::debug!(community = %community_id, stored, "anti-entropy stored heads");
                    rekindle_governance_runtime::rebuild_governance(&host, &community_id).await;
                }
                Err(error) => {
                    tracing::debug!(community = %community_id, %error, "anti-entropy sweep failed");
                }
            }
        }
    }
}
