//! The communities' login network stage (plan E3.1 C2a), one for both
//! hosts: once the transport reports the public internet ready, every
//! community's records are opened and handed over (the leases, watches and
//! keepalive; the shell's per-community loops through
//! `ShellHooks::community_records_ready`), its member state hydrated, its
//! governance rebuilt and its active records republished, then each
//! community's history catch-up is scheduled. It is a task of the unlock's
//! `SessionScope`, so a lock cancels it, and its DHT work is record-pool
//! calls, which the lock's drain releases (plan C7.6g).

use std::sync::Arc;

use rekindle_governance_runtime::dht_hydration;
use rekindle_lifecycle::{ScopeClosed, SessionScope};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

/// The stage's task name in the unlock scope.
pub(crate) const STAGE_TASK: &str = "community network stage";

/// Start the stage in `scope`, waiting on `ready` (the transport's public
/// internet readiness) before any DHT work.
///
/// # Errors
/// The scope closed before the stage started.
pub(crate) fn start(
    governance: HostGovernance,
    mut ready: watch::Receiver<bool>,
    scope: &Arc<SessionScope>,
) -> Result<(), ScopeClosed> {
    scope.spawn_with_token(STAGE_TASK, move |stop| async move {
        let attached = stop
            .run_until_cancelled(async { ready.wait_for(|ready| *ready).await.is_ok() })
            .await;
        // Cancelled (the lock), or the transport went away.
        if attached != Some(true) {
            return;
        }
        run(&governance, &stop).await;
    })
}

/// Open, hydrate, rebuild and republish every community, then schedule
/// each one's history catch-up; nothing further starts once `stop` fires.
async fn run(governance: &HostGovernance, stop: &CancellationToken) {
    dht_hydration::open_community_dht_records(governance).await;
    if stop.is_cancelled() {
        return;
    }
    dht_hydration::hydrate_community_state_from_dht(governance).await;
    // Each rebuilt community's GovernanceRebuilt and MembersRefreshed come
    // from the rebuild itself (plan E3.1 C1).
    dht_hydration::rebuild_governance_from_dht(governance).await;
    if stop.is_cancelled() {
        return;
    }
    dht_hydration::republish_active_records(governance).await;
    if stop.is_cancelled() {
        return;
    }
    let communities: Vec<String> = governance
        .ctx
        .communities
        .map()
        .read()
        .keys()
        .cloned()
        .collect();
    for community_id in &communities {
        governance.spawn_history_catchup_impl(community_id);
    }
    tracing::info!(
        communities = communities.len(),
        "communities hydrated, rebuilt and republished"
    );
}
