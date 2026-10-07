//! A community's session leases in the record pool (plan C7.5).
//!
//! The governance runtime hands every record a community keeps for its
//! session — governance, registry, channels, segments, overflow pages — to
//! [`records_ready`]. This is the one place that holds them, starts the
//! community's loops, and releases them when the member leaves (logout's
//! pool shutdown closes the rest).

use std::sync::Arc;

use rekindle_protocol::dht::pool::RecordPool;
use rekindle_records::lease::{CommunityLeases, HeldKind, LeaseId};

use crate::state::AppState;
use crate::state_helpers;

/// Merge `incoming` into the community's held set. A lease on a record the
/// set already holds is released, so repeat hand-overs never accumulate;
/// so is every lease for a community that is not (or no longer) joined.
pub(crate) async fn hold(state: &Arc<AppState>, community_id: &str, incoming: CommunityLeases) {
    let Ok(pool) = state_helpers::record_pool(state) else {
        // Logged out: the pool is gone and has closed every record.
        return;
    };
    let surplus = merge(state, &pool, community_id, incoming);
    for lease in surplus {
        pool.release(lease).await;
    }
}

/// [`hold`], then watch the community's records and start its inspect,
/// keepalive and presence loops the first time, in the community's scope.
pub(crate) async fn records_ready(
    state: &Arc<AppState>,
    community_id: &str,
    incoming: CommunityLeases,
) {
    hold(state, community_id, incoming).await;
    if let Err(error) =
        crate::services::community::watch::watch_community_records(state, community_id).await
    {
        tracing::debug!(community = %community_id, %error, "failed to watch community records");
    }
    let start_loops = {
        let mut communities = state.communities.write();
        communities
            .get_mut(community_id)
            .is_some_and(|cs| !std::mem::replace(&mut cs.loops_started, true))
    };
    if start_loops {
        crate::services::community::inspect::start_inspect_loop(
            Arc::clone(state),
            community_id.to_string(),
        );
        crate::services::community::presence::start_presence_poll(state, community_id.to_string());
        crate::services::community::keepalive::start_dht_keepalive(
            Arc::clone(state),
            community_id.to_string(),
        );
    }
}

/// Release every lease the community holds (leave). The pool closes a
/// record when its last borrower releases it.
pub(crate) async fn release_all(state: &Arc<AppState>, community_id: &str) {
    let held = {
        let mut communities = state.communities.write();
        communities
            .get_mut(community_id)
            .map(|cs| std::mem::take(&mut cs.leases))
    };
    let (Some(held), Ok(pool)) = (held, state_helpers::record_pool(state)) else {
        return;
    };
    for lease in held.all() {
        pool.release(lease).await;
    }
}

/// Fold `incoming` into the community's leases
/// ([`CommunityLeases::merge`]), recording each newly held record's key in
/// its key inventory (`open_community_records`, which the watch and inspect
/// paths read); returns the surplus to release.
fn merge(
    state: &Arc<AppState>,
    pool: &RecordPool,
    community_id: &str,
    incoming: CommunityLeases,
) -> Vec<LeaseId> {
    let mut communities = state.communities.write();
    let Some(cs) = communities.get_mut(community_id) else {
        return incoming.all().collect();
    };
    let merged = cs.leases.merge(incoming, |lease| pool.key_of(lease));
    let inventory = &mut cs.open_community_records;
    for (kind, key) in merged.added {
        match kind {
            HeldKind::Governance => inventory.governance_key = Some(key),
            HeldKind::Registry => inventory.registry_key = Some(key),
            HeldKind::Channel | HeldKind::Segment => inventory.add_channel(key),
            HeldKind::Overflow => inventory.add_overflow(key),
        }
    }
    merged.surplus
}
