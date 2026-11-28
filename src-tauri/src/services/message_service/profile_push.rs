//! Our own profile push, for the relay pool (E5.2): written through the
//! session's record pool, which holds the profile writable from login (plan
//! C7.4). The friend-list push is the Host's (plan E2.1).

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Push a profile subkey to our profile DHT record.
pub async fn push_profile_update(
    state: &Arc<AppState>,
    subkey: u32,
    value: Vec<u8>,
) -> Result<(), String> {
    let profile_key = state
        .node
        .read()
        .as_ref()
        .ok_or("node not initialized")?
        .profile_dht_key
        .clone()
        .ok_or("no profile DHT key")?;
    let pool = state_helpers::record_pool(state)?;
    let outcome = rekindle_protocol::dht::profile::set_own_profile_subkey(
        pool.as_ref(),
        &profile_key,
        subkey,
        value,
    )
    .await
    .map_err(|e| format!("failed to push profile update: {e}"))?;
    if outcome.missed() {
        tracing::warn!(subkey, ?outcome, "profile update not stored at consensus");
    }
    tracing::debug!(subkey, profile_key = %profile_key, "pushed profile update to DHT");
    Ok(())
}
