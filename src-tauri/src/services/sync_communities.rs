//! A watched community record's change. Nothing here polls (D4): the
//! periodic re-announce is the Host's
//! (`adapters::governance::community_sync`, plan E3.1 C2b), and a governance
//! record's ValueChange rebuilds that one community (plan E3.1 C1).

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Route a watched record's change: a governance record (or a Plate Gate
/// segment's) to the Host's governance arm, which rebuilds that community
/// alone and opens the channel records it newly names (plan E3.1 C1); a
/// channel record to the Host's channel ingest; a registry to a presence
/// tick. `Ok(false)`
/// for a record no community holds; `NotLoggedIn` while locked.
pub(crate) async fn handle_community_record_change(
    state: &Arc<AppState>,
    dht_key: &str,
    subkeys: &[u32],
) -> Result<bool, rekindle_db::NotLoggedIn> {
    if state_helpers::host_governance(state)?
        .on_watched_change(dht_key, subkeys)
        .await
    {
        return Ok(true);
    }
    // A channel record: each changed slot's entries pass the Host's ingest
    // and its D26 gate, as on the daemon (`daemon/handler.rs`).
    if state_helpers::host_channel(state)?
        .on_record_change(dht_key, subkeys)
        .await
    {
        return Ok(true);
    }

    let changed = {
        let communities = state.communities.read();
        communities.values().find_map(|community| {
            if community.member_registry_key.as_deref() == Some(dht_key) {
                return Some(community.id.clone());
            }
            // Plate Gate (§15.4): a segment-N registry change is a presence
            // change, as segment 0's (segments stay until E3.4).
            let gov = community.governance_state.as_ref()?;
            gov.segments
                .iter()
                .any(|s| s.registry_key == dht_key)
                .then(|| community.id.clone())
        })
    };

    // A registry (or segment registry) change is a presence change.
    let Some(community_id) = changed else {
        return Ok(false);
    };
    let _ = crate::services::community::presence_poll_tick_public(state, &community_id).await;
    Ok(true)
}
