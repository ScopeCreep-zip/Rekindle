//! The shell's effects of a governance projection (plan E3.1 B2c): the Host
//! projected merged governance onto the community store
//! (`rekindle_node::host::adapters::governance::projection`), and these are
//! what it cannot run yet. `ShellHooks::on_governance_projected` names the
//! owner step of each.

use rekindle_node::host::adapters::governance::projection::GovernanceProjected;

use crate::state::SharedState;

/// Run the shell's effects of a projection of `community_id`.
pub(super) fn on_projected(
    state: &SharedState,
    community_id: &str,
    projected: GovernanceProjected,
) {
    // Event RSVP lists from the merged events (events domain, E3.1 E3).
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            if let Some(gov) = cs.governance_state.clone() {
                crate::services::community::event_rsvps::rebuild_from_governance(cs, &gov);
            }
        }
    }

    // The Lost Cargo cache and its pinned set (files domain, E3.1 E3).
    super::open_files_cache(state, community_id);
    crate::services::community::files::sync_pinned_from_governance(state, community_id);

    let scope = crate::state_helpers::login_scope_or_closed(state);
    // New Plate Gate segment records opened (the watch path, E3.1 C1).
    let (s, c) = (state.clone(), community_id.to_string());
    scope.spawn_or_drop("open new plate segments", async move {
        crate::services::community::segments::open_new_segments(&s, &c).await;
    });
    // The watches refreshed when the merge named a new record (E3.1 C1).
    if projected.new_watch_targets {
        let (s, c) = (state.clone(), community_id.to_string());
        scope.spawn_or_drop("governance watch refresh", async move {
            if let Err(e) = crate::services::community::watch::watch_community_records(&s, &c).await
            {
                tracing::warn!(community = %c, error = %e, "watches not refreshed after a governance update");
            }
        });
    }
    // Expression assets fetched eagerly (files domain, E3.1 E3).
    let (s, c) = (state.clone(), community_id.to_string());
    scope.spawn_or_drop("eager expression cache", async move {
        crate::services::community::expression_assets::eager_fetch_missing(&s, &c).await;
    });

    // Merged events into `community_events` and the reminders woken
    // (events domain, E3.1 E3).
    if let Ok(pool) = state.db.current() {
        crate::services::community::events_hydration::hydrate_events_from_governance(
            state,
            &pool,
            community_id,
        );
        crate::services::community::wake_event_reminders(state);
    }
}
