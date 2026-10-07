use std::sync::Arc;

use rekindle_sync::gap::GapDetector;
use rekindle_sync::inspect::INSPECT_INTERVAL;

use crate::state::AppState;
use crate::state_helpers;

/// Every record key the community tracks (governance, registry, channels).
/// The background sync after a suspend inspects all of them: watches may
/// have lapsed while the process was suspended.
pub(crate) fn tracked_record_keys(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<Vec<String>> {
    let communities = state.communities.read();
    let records = &communities.get(community_id)?.open_community_records;
    Some(
        records
            .governance_key
            .iter()
            .chain(records.registry_key.iter())
            .chain(records.channel_keys.iter())
            .cloned()
            .collect(),
    )
}

/// The tracked records that have no active watch. Only these are inspected
/// each tick: a watched record already gets Veilid's own fallback inspect
/// every 30 s, so inspecting it here too only duplicated that (plan C7, v1
/// R10).
pub(crate) fn unwatched_record_keys(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<Vec<String>> {
    let tracked = tracked_record_keys(state, community_id)?;
    let communities = state.communities.read();
    let watched = &communities.get(community_id)?.watched_records;
    Some(
        tracked
            .into_iter()
            .filter(|key| !watched.contains(key))
            .collect(),
    )
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
    let record_pool = state_helpers::record_pool(state)?;
    let parsed_key = record_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid record key: {e}"))?;
    let report = record_pool
        .inspect_once(&parsed_key, None, veilid_core::DHTReportScope::SyncGet)
        .await
        .map_err(|e| format!("inspect failed: {e}"))?;

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

    // A8 telemetry: how often does the poll surface something the watch
    // path had not already delivered? INSPECT_INTERVAL is only relaxed
    // against this measured miss rate (see rekindle_sync::inspect).
    rekindle_sync::inspect::INSPECT_TELEMETRY.record(!changed_subkeys.is_empty());

    if changed_subkeys.is_empty() {
        return Ok(());
    }
    tracing::debug!(
        community = %community_id,
        record_key = %record_key,
        subkeys = changed_subkeys.len(),
        "inspect surfaced changes the watch path missed"
    );

    let pool = state.db.current()?;

    changed_subkeys.sort_unstable();
    changed_subkeys.dedup();

    for subkey in changed_subkeys {
        record_pool
            .read_once(&parsed_key, subkey, true)
            .await
            .map_err(|e| format!("get_dht_value failed during inspect sync: {e}"))?;
        if !crate::services::sync_communities::handle_community_record_change(
            state, &pool, record_key,
        )
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
    let scope = crate::state_helpers::community_scope(&state, &community_id);
    scope.spawn_with_token_or_drop("community inspect", |stop| async move {
        let mut interval = tokio::time::interval(INSPECT_INTERVAL);
        interval.tick().await;

        let mut ticks: u64 = 0;
        loop {
            if stop.run_until_cancelled(interval.tick()).await.is_none() {
                return;
            }
            ticks += 1;
            // Periodic miss-rate summary (A8): the number that decides
            // whether INSPECT_INTERVAL can be relaxed post-0.5.7.
            if ticks.is_multiple_of(10) {
                let (clean, missed) = rekindle_sync::inspect::INSPECT_TELEMETRY.counts();
                tracing::debug!(clean, missed, "inspect catch-up telemetry");
            }

            // Hold any tracked record the community does not hold yet (a
            // failed open heals here). A dead watch is the record pool's
            // to re-arm, not this loop's.
            super::watch::hold_unheld_records(&state, &community_id, &stop).await;

            let Some(unwatched) = unwatched_record_keys(&state, &community_id) else {
                return;
            };

            for record_key in unwatched {
                // Leaving the community stops this; the pool runs the
                // inspect's calls on its own scope, so none is dropped (C4.L1).
                let Some(result) = stop
                    .run_until_cancelled(inspect_record(&state, &community_id, &record_key))
                    .await
                else {
                    return;
                };
                if let Err(e) = result {
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
