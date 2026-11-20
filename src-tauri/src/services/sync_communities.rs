//! A watched community record's change. Nothing here polls (D4): the
//! periodic re-announce is the Host's
//! (`adapters::governance::community_sync`, plan E3.1 C2b), and a governance
//! record's ValueChange rebuilds that one community (plan E3.1 C1).

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Route a watched record's change: a governance record (or a Plate Gate
/// segment's) to the Host's governance arm, which rebuilds that community
/// alone (plan E3.1 C1); a registry to a presence refresh, which reads the
/// rows and writes nothing (channel posts
/// ride their authors' `M`/4 outboxes, read by the ring's watch, plan
/// E3.3). `Ok(false)` for a record no community holds; `NotLoggedIn` while locked.
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

    // A registry (or segment registry) change is a presence change: read
    // the rows, never write ours (the beat is the poll's; finding IB1c).
    let Some(community_id) = changed else {
        return Ok(false);
    };
    if let Err(error) =
        crate::services::community::presence::presence_refresh(state, &community_id).await
    {
        tracing::debug!(community = %community_id, %error, "presence refresh after a registry change failed");
    }
    Ok(true)
}
