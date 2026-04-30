use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::db::DbPool;
use crate::services::{presence_service, sync_service};
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
                community.governance_key.as_deref() == Some(dht_key)
                    || community.member_registry_key.as_deref() == Some(dht_key)
                    || community
                        .channel_log_keys
                        .values()
                        .any(|key| key == dht_key)
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

    if sync_service::handle_community_record_change(state, pool.inner(), &key).await {
        tracing::debug!(key = %key, "handled community DHT change via sync service");
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
    }
}
