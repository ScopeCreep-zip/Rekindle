use std::sync::Arc;

use rekindle_sync::watch::WatchManager;

use crate::state::AppState;
use crate::state_helpers;

pub fn mark_watch_active(state: &Arc<AppState>, community_id: &str, record_key: &str) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        let mut watch_manager = WatchManager::default();
        for watched_key in &community.watched_records {
            watch_manager.mark_active(watched_key.clone());
        }
        watch_manager.mark_active(record_key.to_string());
        community.watched_records.insert(record_key.to_string());
    }
}

pub fn mark_watch_inactive(state: &Arc<AppState>, record_key: &str) {
    let mut communities = state.communities.write();
    for community in communities.values_mut() {
        let mut watch_manager = WatchManager::default();
        for watched_key in &community.watched_records {
            watch_manager.mark_active(watched_key.clone());
        }
        watch_manager.mark_inactive(record_key);
        community.watched_records.remove(record_key);
    }
}

async fn watch_record(
    rc: &veilid_core::RoutingContext,
    state: &Arc<AppState>,
    community_id: &str,
    label: &str,
    record_key: &str,
) {
    let Ok(parsed_key) = record_key.parse::<veilid_core::RecordKey>() else {
        tracing::debug!(
            community = %community_id,
            label,
            record_key,
            "skipping watch for invalid record key"
        );
        return;
    };
    match rc
        .watch_dht_values(
            parsed_key,
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            None,
            None,
        )
        .await
    {
        Ok(true) => {
            mark_watch_active(state, community_id, record_key);
            tracing::debug!(
                community = %community_id,
                label,
                record_key,
                "watching community record"
            );
        }
        Ok(false) => {
            tracing::warn!(
                community = %community_id,
                label,
                record_key,
                "community watch did not become active"
            );
        }
        Err(e) => {
            tracing::warn!(
                community = %community_id,
                label,
                record_key,
                error = %e,
                "failed to watch community record"
            );
        }
    }
}

pub async fn watch_community_records(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(), String> {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return Err("not attached".into());
    };
    let (governance_key, registry_key, channel_keys) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        (
            community.open_community_records.governance_key.clone(),
            community.open_community_records.registry_key.clone(),
            community.open_community_records.channel_keys.clone(),
        )
    };

    if let Some(governance_key) = governance_key {
        watch_record(&rc, state, community_id, "governance", &governance_key).await;
    }
    if let Some(registry_key) = registry_key {
        watch_record(&rc, state, community_id, "registry", &registry_key).await;
    }
    for channel_key in &channel_keys {
        watch_record(&rc, state, community_id, "channel", channel_key).await;
    }
    Ok(())
}
