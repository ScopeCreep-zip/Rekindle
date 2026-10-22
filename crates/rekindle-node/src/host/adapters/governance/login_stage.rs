//! The communities' login network stage (plan E3.1 C2a), one for both
//! hosts: once the transport reports the public internet ready, each
//! community, in id order, has its records opened and handed over (the
//! leases, watches and keepalive; the shell's per-community loops through
//! `ShellHooks::community_records_ready`), its member state hydrated, its
//! governance rebuilt and its history catch-up scheduled; then our active
//! records are republished. It is a task of the unlock's `SessionScope`, so
//! a lock cancels it, and its DHT work is record-pool calls, which the
//! lock's drain releases (plan C7.6g).
//!
//! The same stage heals (plan E3.1 C2c), on both modes: each later return
//! of readiness (a reconnect) re-runs it, and so does a wake
//! ([`HostGovernance::heal_communities`]).

use std::sync::Arc;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::dht_hydration;
use rekindle_lifecycle::{ScopeClosed, SessionScope};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

/// The stage's task name in the unlock scope.
pub(crate) const STAGE_TASK: &str = "community network stage";

/// Start the stage in `scope`, waiting on `ready` (the transport's public
/// internet readiness) before any DHT work, and re-running it on each
/// later rising edge of `ready` (a reconnect) until the scope closes.
///
/// # Errors
/// The scope closed before the stage started.
pub(crate) fn start(
    governance: HostGovernance,
    mut ready: watch::Receiver<bool>,
    scope: &Arc<SessionScope>,
) -> Result<(), ScopeClosed> {
    scope.spawn_with_token(STAGE_TASK, move |stop| async move {
        // Cancelled (the lock), or the transport went away.
        if !becomes(&mut ready, true, &stop).await {
            return;
        }
        run(&governance, &stop).await;
        while becomes(&mut ready, false, &stop).await && becomes(&mut ready, true, &stop).await {
            tracing::info!("network reconnected; healing the communities");
            run(&governance, &stop).await;
        }
    })
}

/// Wait until `ready` reads `wanted`; false when `stop` fired or the
/// transport went away first.
async fn becomes(
    ready: &mut watch::Receiver<bool>,
    wanted: bool,
    stop: &CancellationToken,
) -> bool {
    stop.run_until_cancelled(async { ready.wait_for(|ready| *ready == wanted).await.is_ok() })
        .await
        == Some(true)
}

impl HostGovernance {
    /// Re-run the stage now (a wake): every community healed as at login.
    /// The caller runs it in the unlock scope, which a lock cancels.
    pub async fn heal_communities(&self) {
        run(self, &CancellationToken::new()).await;
    }
}

/// The per-community stage for every community, in id order, then the
/// republish of our active records; nothing further starts once `stop`
/// fires.
async fn run(governance: &HostGovernance, stop: &CancellationToken) {
    let mut communities: Vec<String> = governance
        .ctx
        .communities
        .map()
        .read()
        .keys()
        .cloned()
        .collect();
    communities.sort();
    for community_id in &communities {
        if stop.is_cancelled() {
            return;
        }
        community(governance, community_id, stop).await;
    }
    if stop.is_cancelled() {
        return;
    }
    dht_hydration::republish_active_records(governance).await;
    tracing::info!(
        communities = communities.len(),
        "communities hydrated, rebuilt and republished"
    );
}

/// One community's stage: open and hand over its records, hydrate its
/// member state, rebuild its governance (its GovernanceRebuilt and
/// MembersRefreshed come from the rebuild itself, plan E3.1 C1), and
/// schedule its history catch-up.
async fn community(governance: &HostGovernance, community_id: &str, stop: &CancellationToken) {
    let setup = governance
        .list_communities_for_dht_open()
        .into_iter()
        .find(|setup| setup.id == community_id);
    if let Some(setup) = setup {
        dht_hydration::open_and_track_one_community(governance, &setup).await;
    }
    if stop.is_cancelled() {
        return;
    }
    dht_hydration::hydrate_one_community(governance, community_id).await;
    dht_hydration::rebuild_governance(governance, community_id).await;
    if stop.is_cancelled() {
        return;
    }
    governance.spawn_history_catchup_impl(community_id);
}
