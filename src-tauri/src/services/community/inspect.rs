use std::sync::Arc;

use rekindle_sync::gap::GapDetector;
use rekindle_sync::inspect::INSPECT_INTERVAL;
use tauri::Manager;

use crate::state::AppState;
use crate::state_helpers;

pub(crate) fn tracked_record_keys(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<Vec<String>> {
    let communities = state.communities.read();
    let community = communities.get(community_id)?;
    let mut keys = Vec::new();
    if let Some(key) = community.open_community_records.governance_key.clone() {
        keys.push(key);
    }
    if let Some(key) = community.open_community_records.registry_key.clone() {
        keys.push(key);
    }
    keys.extend(community.open_community_records.channel_keys.clone());
    Some(keys)
}

fn changed_subkeys_from_sequences(local_sequences: &[u64], network_sequences: &[u64]) -> Vec<u32> {
    GapDetector::detect(local_sequences, network_sequences)
        .into_iter()
        .filter_map(|gap| u32::try_from(gap.subkey).ok())
        .collect()
}

pub(crate) async fn inspect_record(
    state: &Arc<AppState>,
    community_id: &str,
    record_key: &str,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let parsed_key = record_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid record key: {e}"))?;
    let report = rc
        .inspect_dht_record(
            parsed_key.clone(),
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            veilid_core::DHTReportScope::SyncGet,
        )
        .await
        .map_err(|e| format!("inspect_dht_record failed: {e}"))?;

    let mut changed_subkeys = {
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(community_id)
            .ok_or("community not found during inspect")?;
        let previous = community
            .record_sequences
            .entry(record_key.to_string())
            .or_insert_with(Vec::new);
        let local_sequences: Vec<u64> = previous
            .iter()
            .map(|seq| u64::from(u32::from(*seq)))
            .collect();
        let network_sequences: Vec<u64> = report
            .network_seqs()
            .iter()
            .map(|seq| u64::from(u32::from(*seq)))
            .collect();
        changed_subkeys_from_sequences(&local_sequences, &network_sequences)
    };

    if changed_subkeys.is_empty() {
        return Ok(());
    }

    let pool = state_helpers::app_handle(state)
        .and_then(|app| {
            app.try_state::<crate::db::DbPool>()
                .map(|pool| pool.inner().clone())
        })
        .ok_or("db pool not available for inspect loop")?;

    changed_subkeys.sort_unstable();
    changed_subkeys.dedup();

    for subkey in changed_subkeys {
        rc.get_dht_value(parsed_key.clone(), subkey, true)
            .await
            .map_err(|e| format!("get_dht_value failed during inspect sync: {e}"))?;
        if !crate::services::sync_service::handle_community_record_change(state, &pool, record_key)
            .await
        {
            continue;
        }

        let network_seq = report
            .network_seqs()
            .get(subkey as usize)
            .copied()
            .unwrap_or_default();
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(community_id)
            .ok_or("community missing while updating inspect sequences")?;
        let previous = community
            .record_sequences
            .entry(record_key.to_string())
            .or_insert_with(Vec::new);
        if previous.len() <= subkey as usize {
            previous.resize(subkey as usize + 1, veilid_core::ValueSeqNum::default());
        }
        previous[subkey as usize] = network_seq;
    }

    Ok(())
}

pub fn start_inspect_loop(state: Arc<AppState>, community_id: String) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(INSPECT_INTERVAL);
        interval.tick().await;

        loop {
            interval.tick().await;

            // W-1 #16 — re-attempt watches on any tracked record whose
            // previous watch died (Veilid renew_watch returned false,
            // OR establish-time `Ok(false)` at first attempt). Without
            // this, dead watches stayed dead until the next value-
            // change event, which by definition cannot fire on a dead
            // watch — meaning some records would silently lose their
            // notification stream until the user restarted.
            super::watch::retry_dead_watches(&state, &community_id).await;

            let Some(tracked_records) = tracked_record_keys(&state, &community_id) else {
                return;
            };

            for record_key in tracked_records {
                if let Err(e) = inspect_record(&state, &community_id, &record_key).await {
                    tracing::debug!(
                        community = %community_id,
                        record_key = %record_key,
                        error = %e,
                        "community inspect tick failed"
                    );
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod tests;
