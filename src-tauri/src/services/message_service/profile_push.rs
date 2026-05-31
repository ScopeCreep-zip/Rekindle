//! Phase 23.D — DHT profile + friend-list push helpers lifted from
//! `message_service/mod.rs`. Each function opens our own DHT record
//! with the owner keypair then calls `set_dht_value`. Pure Veilid
//! orchestration — no protocol logic per Invariant 7.

use std::sync::Arc;

use crate::state::AppState;

/// Push a profile subkey to our profile DHT record.
pub async fn push_profile_update(
    state: &Arc<AppState>,
    subkey: u32,
    value: Vec<u8>,
) -> Result<(), String> {
    let (profile_key, routing_context, owner_keypair) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let pk = nh.profile_dht_key.clone().ok_or("no profile DHT key")?;
        (
            pk,
            nh.routing_context.clone(),
            nh.profile_owner_keypair.clone(),
        )
    };

    let record_key: veilid_core::RecordKey = profile_key
        .parse()
        .map_err(|e| format!("invalid profile key: {e}"))?;

    // Ensure the record is open with write access before writing.
    // Re-opening an already-open record is a no-op in Veilid.
    let _ = routing_context
        .open_dht_record(record_key.clone(), owner_keypair)
        .await
        .map_err(|e| format!("failed to open profile record for push: {e}"))?;

    routing_context
        .set_dht_value(record_key, subkey, value, None)
        .await
        .map_err(|e| format!("failed to push profile update: {e}"))?;

    tracing::debug!(subkey, profile_key = %profile_key, "pushed profile update to DHT");
    Ok(())
}

/// Push the local friend list to our DHT friend list record.
///
/// Serializes the current friend public keys as a JSON array and writes
/// it to our friend list DHT record (subkey 0).
pub async fn push_friend_list_update(state: &Arc<AppState>) -> Result<(), String> {
    let (friend_list_key, routing_context, owner_keypair, friend_keys) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let flk = nh
            .friend_list_dht_key
            .clone()
            .ok_or("no friend list DHT key")?;
        let rc = nh.routing_context.clone();
        let kp = nh.friend_list_owner_keypair.clone();
        let friends = state.friends.read();
        let keys: Vec<String> = friends
            .iter()
            .filter(|(_, f)| !matches!(f.friendship_state, crate::state::FriendshipState::Removing))
            .map(|(k, _)| k.clone())
            .collect();
        (flk, rc, kp, keys)
    };

    let record_key: veilid_core::RecordKey = friend_list_key
        .parse()
        .map_err(|e| format!("invalid friend list key: {e}"))?;

    // Ensure the record is open with write access before writing.
    let _ = routing_context
        .open_dht_record(record_key.clone(), owner_keypair)
        .await
        .map_err(|e| format!("failed to open friend list record for push: {e}"))?;

    let value =
        serde_json::to_vec(&friend_keys).map_err(|e| format!("serialize friend list: {e}"))?;

    routing_context
        .set_dht_value(record_key, 0, value, None)
        .await
        .map_err(|e| format!("failed to push friend list update: {e}"))?;

    tracing::debug!(
        friend_list_key = %friend_list_key,
        count = friend_keys.len(),
        "pushed friend list update to DHT"
    );
    Ok(())
}
