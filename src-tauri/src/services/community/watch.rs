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

/// Every community record key the watch tier tracks, labelled for
/// logging: top-level governance + member registry + channel logs from
/// `open_community_records`, PLUS Plate Gate segment-N governance /
/// registry records and channel-segment records from the merged
/// governance state (§15.4). The presence scan reads segment
/// registries, so they must be watched like segment 0 — otherwise a
/// >255-member community's presence updates only surface on the poll
/// backstop. De-duped (segment lists can overlap the top-level keys).
fn tracked_watch_keys(community: &crate::state::CommunityState) -> Vec<(&'static str, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut push =
        |label: &'static str, key: String, seen: &mut std::collections::HashSet<String>| {
            if !key.is_empty() && seen.insert(key.clone()) {
                out.push((label, key));
            }
        };
    if let Some(ref gov_key) = community.open_community_records.governance_key {
        push("governance", gov_key.clone(), &mut seen);
    }
    if let Some(ref reg_key) = community.open_community_records.registry_key {
        push("registry", reg_key.clone(), &mut seen);
    }
    for ch_key in &community.open_community_records.channel_keys {
        push("channel", ch_key.clone(), &mut seen);
    }
    if let Some(gov) = community.governance_state.as_ref() {
        for seg in &gov.segments {
            push("segment-governance", seg.governance_key.clone(), &mut seen);
            push("segment-registry", seg.registry_key.clone(), &mut seen);
        }
        for csr in gov.channel_segment_records.values() {
            push("channel-segment", csr.record_key.clone(), &mut seen);
        }
    }
    out
}

/// W-1 #16 — return the set of tracked community record keys that are
/// currently NOT in `watched_records`. The inspect loop calls this each
/// tick and attempts to re-establish those watches; this turns watch
/// death (Veilid `renew_watch == false`) from a silent observability
/// hole into an actively-retried recovery.
pub fn unwatched_tracked_records(state: &Arc<AppState>, community_id: &str) -> Vec<String> {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return Vec::new();
    };
    tracked_watch_keys(community)
        .into_iter()
        .filter(|(_, key)| !community.watched_records.contains(key))
        .map(|(_, key)| key)
        .collect()
}

/// W-1 #16 — re-attempt watches on any tracked record whose previous
/// watch died (`Ok(false)` at establish OR `count == 0` value-change).
/// Called from the inspect loop tick. Idempotent: records already
/// watched are skipped via the `unwatched_tracked_records` filter.
pub async fn retry_dead_watches(state: &Arc<AppState>, community_id: &str) {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return;
    };
    let pending = unwatched_tracked_records(state, community_id);
    if pending.is_empty() {
        return;
    }
    for record_key in pending {
        watch_record(&rc, state, community_id, "retry", &record_key).await;
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
    let (records_open, keys) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        (
            community.open_community_records.records_open,
            tracked_watch_keys(community),
        )
    };
    // Record keys are restored from persistence at resume, but Veilid
    // requires open_dht_record THIS session before watch_dht_values —
    // a pre-open watch can only fail with "record not open". Skip it:
    // `open_one_community_dht_records` always issues the watch after
    // the session's opens complete, covering this same key list.
    if !records_open {
        tracing::debug!(
            community = %community_id,
            "watch requested before this session's record opens — post-open watch covers it"
        );
        return Ok(());
    }

    for (label, key) in &keys {
        watch_record(&rc, state, community_id, label, key).await;
    }
    Ok(())
}
