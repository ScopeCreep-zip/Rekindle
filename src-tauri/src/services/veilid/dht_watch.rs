use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::db::DbPool;
use crate::services::presence_service;
use crate::state::AppState;

async fn try_rewatch_friend(state: &Arc<AppState>, dht_key: &str) {
    let friend_info = {
        crate::state_helpers::friend_for_dht_key(state, dht_key).map(|fk| (fk, dht_key.to_string()))
    };
    let Some((friend_key, record_key)) = friend_info else {
        return;
    };
    if presence_service::watch_friend(state, &friend_key, &record_key)
        .await
        .is_err()
    {
        state.unwatched_friends.write().insert(friend_key);
    }
}

async fn try_rewatch_community(state: &Arc<AppState>, dht_key: &str) {
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|community| {
                if community.governance_key.as_deref() == Some(dht_key)
                    || community.member_registry_key.as_deref() == Some(dht_key)
                    || community
                        .channel_log_keys
                        .values()
                        .any(|key| key == dht_key)
                {
                    return true;
                }
                // Plate Gate (architecture §15.4): also match segment-N
                // governance / registry / channel records.
                if let Some(gov) = community.governance_state.as_ref() {
                    if gov.segments.iter().any(|s| {
                        s.governance_key == dht_key || s.registry_key == dht_key
                    }) {
                        return true;
                    }
                    if gov
                        .channel_segment_records
                        .values()
                        .any(|csr| csr.record_key == dht_key)
                    {
                        return true;
                    }
                }
                false
            })
            .map(|community| community.id.clone())
    };
    let Some(community_id) = community_id else {
        return;
    };
    if let Err(error) =
        crate::services::community::watch_community_records(state, &community_id).await
    {
        tracing::debug!(
            community = %community_id,
            dht_key,
            error = %error,
            "failed to re-watch governance community records"
        );
    }
}

pub async fn handle_value_change(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    change: veilid_core::VeilidValueChange,
) {
    let key = change.key.to_string();

    // Phase 7 — watch-tier trigger. If the ValueChange is on the
    // local user's mailbox key (where peers write friend requests),
    // wake the friendship coordinator so it scans within ~500 ms
    // instead of waiting for the 30 s poll backstop. The helper
    // honours the watch trigger's dev-disable deadline (kill-switch)
    // and is a no-op when no coordinator is running.
    let own_mailbox_key = state
        .node
        .read()
        .as_ref()
        .and_then(|nh| nh.mailbox_dht_key.clone());
    if own_mailbox_key.as_deref() == Some(key.as_str()) {
        state.friendship_handle.fire_watch_trigger();
    }

    if change.subkeys.is_empty() {
        crate::services::community::mark_watch_inactive(state, &key);
        tracing::warn!(key = %key, count = change.count, "DHT watch died; attempting immediate re-watch");
        try_rewatch_friend(state, &key).await;
        try_rewatch_community(state, &key).await;
        return;
    }

    if change.count == 0 {
        crate::services::community::mark_watch_inactive(state, &key);
        tracing::info!(key = %key, "DHT watch expiring (count=0); attempting immediate re-watch");
        try_rewatch_friend(state, &key).await;
        try_rewatch_community(state, &key).await;
    }

    let subkeys: Vec<u32> = change.subkeys.iter().collect();
    let first_subkey = subkeys.first().copied();
    let inline_value = change.value.as_ref().map(|v| v.data().to_vec());
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    tracing::debug!(
        key = %key,
        subkeys = ?subkeys,
        has_inline = inline_value.is_some(),
        "DHT value changed"
    );

    let routing_context = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.routing_context.clone())
    };

    if crate::services::sync_communities::handle_community_record_change(state, pool.inner(), &key).await {
        tracing::debug!(key = %key, "handled community DHT change via sync service");
        return;
    }

    // Personal cross-device sync record (architecture §28.4).
    if crate::services::cross_device_sync::watch::try_handle_personal_sync_change(
        app_handle,
        state,
        pool.inner(),
        &key,
        &subkeys,
        inline_value.as_deref(),
    )
    .await
    {
        return;
    }

    // DM SMPL records: try the DM dispatcher first. Returns true when the
    // key matches a row in `dms`. (Architecture §27 — DMs reuse the SMPL
    // schema universally; the watch goes through the same plumbing as
    // community records.)
    if try_handle_dm_change(state, pool.inner(), &key, &subkeys, routing_context.as_ref())
        .await
    {
        return;
    }

    for &subkey in &subkeys {
        let use_inline = Some(subkey) == first_subkey;
        let value = if use_inline && inline_value.is_some() {
            inline_value.clone().unwrap_or_default()
        } else if let Some(ref rc) = routing_context {
            match rc.get_dht_value(change.key.clone(), subkey, true).await {
                Ok(Some(v)) => v.data().to_vec(),
                Ok(None) => {
                    tracing::debug!(subkey, key = %key, "subkey has no value");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(subkey, key = %key, error = %e, "failed to fetch subkey");
                    continue;
                }
            }
        } else {
            tracing::debug!(subkey, "no routing context to fetch subkey value");
            continue;
        };
        presence_service::handle_value_change(app_handle, state, &key, &[subkey], &value);
        // Mutual Aid (architecture §14.3): we hold a watch slot for this
        // record, so gossip-relay the notification to community peers
        // who may not — the receivers fetch the new value themselves
        // via `get_dht_value` (no ciphertext crosses gossip).
        relay_watch_change(state, &key, subkey, &value);
    }
}

async fn try_handle_dm_change(
    state: &Arc<AppState>,
    pool: &crate::db::DbPool,
    record_key: &str,
    subkeys: &[u32],
    routing_context: Option<&veilid_core::RoutingContext>,
) -> bool {
    use crate::db_helpers::db_call_or_default;
    use crate::state_helpers;

    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return false;
    }
    let owner = owner_key;
    let record = record_key.to_string();
    let exists: bool = db_call_or_default(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM dms WHERE owner_key = ?1 AND record_key = ?2 LIMIT 1",
                rusqlite::params![owner, record],
                |_| Ok(()),
            )
            .is_ok())
    })
    .await;
    if !exists {
        return false;
    }

    let Some(rc) = routing_context else {
        return true;
    };
    let Ok(parsed) = record_key.parse::<veilid_core::RecordKey>() else {
        return true;
    };
    for &subkey in subkeys {
        if let Ok(Some(value)) = rc.get_dht_value(parsed.clone(), subkey, true).await {
            if let Err(e) = crate::services::dm::handle_dm_subkey_change(
                state,
                pool,
                record_key,
                subkey,
                value.data(),
            )
            .await
            {
                tracing::debug!(record_key, subkey, error = %e, "dm subkey handler dropped");
            }
        }
    }
    true
}

fn relay_watch_change(state: &Arc<AppState>, record_key: &str, subkey: u32, value: &[u8]) {
    use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

    let community_id_and_pseudonym = {
        let communities = state.communities.read();
        communities.values().find_map(|cs| {
            let matched = cs.governance_key.as_deref() == Some(record_key)
                || cs.member_registry_key.as_deref() == Some(record_key)
                || cs.channel_log_keys.values().any(|k| k == record_key)
                || cs.governance_state.as_ref().is_some_and(|gov| {
                    gov.segments
                        .iter()
                        .any(|s| s.governance_key == record_key || s.registry_key == record_key)
                        || gov
                            .channel_segment_records
                            .values()
                            .any(|csr| csr.record_key == record_key)
                });
            matched.then(|| (cs.id.clone(), cs.my_pseudonym_key.clone().unwrap_or_default()))
        })
    };
    let Some((community_id, observer)) = community_id_and_pseudonym else {
        return;
    };
    if observer.is_empty() {
        return;
    }
    let envelope = CommunityEnvelope::WatchRelay {
        record_key: record_key.to_string(),
        subkey,
        content_hash: blake3::hash(value).to_hex().to_string(),
        observer_pseudonym: observer,
    };
    if let Err(e) = crate::services::community::gossip::send_to_mesh(state, &community_id, &envelope) {
        tracing::debug!(community = %community_id, error = %e, "watch relay gossip failed");
    }
}
