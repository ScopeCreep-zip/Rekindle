//! A watched community record's change. Nothing here polls (D4): the
//! periodic re-announce is the Host's
//! (`adapters::governance::community_sync`, plan E3.1 C2b). The shell only
//! forwards: the Host decides what a change means (plan E3.4 S5, ADR 0010),
//! a ring member record's to its governance arm, a registry's to a presence
//! refresh that writes nothing (`registry_watch`, finding IB1c).

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Forward a watched record's change to the Host's governance arm: `Ok(true)`
/// when a community holds the record (a ring member record or a registry),
/// `Ok(false)` for any other record; `NotLoggedIn` while locked.
pub(crate) async fn handle_community_record_change(
    state: &Arc<AppState>,
    dht_key: &str,
    subkeys: &[u32],
) -> Result<bool, rekindle_db::NotLoggedIn> {
    Ok(state_helpers::host_governance(state)?
        .on_watched_change(dht_key, subkeys)
        .await)
}
