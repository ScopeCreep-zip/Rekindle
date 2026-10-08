//! Phase 23.D — DHT profile + friend-list push helpers lifted from
//! `message_service/mod.rs`. Both write our own records through the
//! session's record pool, which holds them writable from login (plan
//! C7.4). Pure Veilid orchestration — no protocol logic per Invariant 7.

use std::sync::Arc;

use rekindle_codec::friend::FriendEntry;

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
    let outcome =
        rekindle_protocol::dht::profile::set_own_profile_subkey(&pool, &profile_key, subkey, value)
            .await
            .map_err(|e| format!("failed to push profile update: {e}"))?;
    if outcome.missed() {
        tracing::warn!(subkey, ?outcome, "profile update not stored at consensus");
    }
    tracing::debug!(subkey, profile_key = %profile_key, "pushed profile update to DHT");
    Ok(())
}

/// Push the local friend list to our DHT friend list record.
///
/// Writes the Cap'n Proto `FriendEntry` list both tracks read
/// (`rekindle_protocol::dht::friends`). Entries already in the record keep
/// their `added_at` and `dm_log_key`; friends being removed are dropped.
pub async fn push_friend_list_update(state: &Arc<AppState>) -> Result<(), String> {
    let friend_list_key = state
        .node
        .read()
        .as_ref()
        .ok_or("node not initialized")?
        .friend_list_dht_key
        .clone()
        .ok_or("no friend list DHT key")?;
    let pool = state_helpers::record_pool(state)?;
    let current = rekindle_protocol::dht::friends::read_friend_list(&pool, &friend_list_key)
        .await
        .map_err(|e| format!("read friend list: {e}"))?;
    let entries: Vec<FriendEntry> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter(|f| !matches!(f.friendship_state, crate::state::FriendshipState::Removing))
            .map(|f| {
                let existing = current
                    .friends
                    .iter()
                    .find(|e| e.public_key == f.public_key);
                FriendEntry {
                    public_key: f.public_key.clone(),
                    nickname: f.nickname.clone(),
                    group: f.group.clone(),
                    added_at: existing.map_or_else(rekindle_utils::timestamp_ms, |e| e.added_at),
                    profile_dht_key: f.dht_record_key.clone(),
                    dm_log_key: existing.and_then(|e| e.dm_log_key.clone()),
                }
            })
            .collect()
    };

    let outcome =
        rekindle_protocol::dht::friends::write_friend_list(&pool, &friend_list_key, &entries)
            .await
            .map_err(|e| format!("failed to push friend list update: {e}"))?;
    if outcome.missed() {
        tracing::warn!(?outcome, "friend list update not stored at consensus");
    }
    tracing::debug!(
        friend_list_key = %friend_list_key,
        count = entries.len(),
        ?outcome,
        "pushed friend list update to DHT"
    );
    Ok(())
}
