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
    // Channel logs come straight from governance-synced state, not from
    // the opened-records list: a channel discovered in a governance
    // rebuild is a watch target *before* anything has opened it, and
    // `watch_record` now heals the open itself. Routing these through
    // `open_community_records.channel_keys` (as an overwrite in
    // `apply_governance_rebuild_result` once did) made that list lie
    // about what was open, which disabled `open_new_channel_records`'
    // already-opened skip check.
    for ch_key in community.channel_log_keys.values() {
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

/// Tracked community records the community does not hold yet (a login open
/// that failed on a cold network, a channel discovered after hydration).
fn unheld_tracked_records(state: &Arc<AppState>, community_id: &str) -> Vec<String> {
    let Ok(pool) = state_helpers::record_pool(state) else {
        return Vec::new();
    };
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return Vec::new();
    };
    let held: std::collections::HashSet<String> = community
        .leases
        .all()
        .filter_map(|lease| pool.key_of(lease))
        .collect();
    tracked_watch_keys(community)
        .into_iter()
        .map(|(_, key)| key)
        .filter(|key| !held.contains(key))
        .collect()
}

/// Borrow every tracked record the community does not hold and hand the
/// leases to it (`leases::records_ready`: held, recorded, watched). Called
/// from the inspect tick, so a record whose open failed heals on a later
/// tick. Watch *death* is not handled here: the record pool re-arms a dead
/// watch on a held lease (plan C7, one watch-death owner). Stops before the
/// next borrow once `stop` is cancelled.
pub async fn hold_unheld_records(
    state: &Arc<AppState>,
    community_id: &str,
    stop: &tokio_util::sync::CancellationToken,
) {
    let pending = unheld_tracked_records(state, community_id);
    if pending.is_empty() {
        return;
    }
    let Ok(pool) = state_helpers::record_pool(state) else {
        return;
    };
    let (governance_key, registry_key, registry_writer) = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            return;
        };
        (
            cs.governance_key.clone(),
            cs.member_registry_key.clone(),
            cs.registry_owner_keypair
                .clone()
                .or_else(|| cs.slot_keypair.clone())
                .and_then(|w| w.parse::<veilid_core::KeyPair>().ok()),
        )
    };
    let mut leases = rekindle_records::lease::CommunityLeases::default();
    for record_key in pending {
        // Leaving the community stops this before the next borrow; a borrow
        // in flight runs on the pool's scope, so none is dropped (C4.L1).
        let Ok(parsed) = record_key.parse::<veilid_core::RecordKey>() else {
            continue;
        };
        let is_registry = registry_key.as_deref() == Some(record_key.as_str());
        let writer = is_registry.then(|| registry_writer.clone()).flatten();
        let Some(acquired) = stop
            .run_until_cancelled(pool.acquire(&parsed, writer))
            .await
        else {
            break;
        };
        match acquired {
            Ok(lease) if governance_key.as_deref() == Some(record_key.as_str()) => {
                leases.governance = Some(lease);
            }
            Ok(lease) if is_registry => leases.registry = Some(lease),
            Ok(lease) => leases.segments.push(lease),
            Err(e) => tracing::debug!(
                community = %community_id,
                record_key,
                error = %e,
                "tracked record not open yet — retrying next tick"
            ),
        }
    }
    if leases.all().next().is_some() {
        crate::services::community::leases::records_ready(state, community_id, leases).await;
    }
}

/// Watch every record the community holds (governance, registry, channels,
/// segments) on its lease. Overflow pages are never watched: the primary
/// subkey's watch re-follows the chain. A watch rides a held lease, so the
/// record is open with its sticky writer and there is nothing to re-open;
/// a watch that dies is re-armed by the record pool.
pub async fn watch_community_records(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(), String> {
    let pool = state_helpers::record_pool(state)?;
    let leases: Vec<rekindle_records::lease::LeaseId> = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        let held = &community.leases;
        held.governance
            .iter()
            .chain(held.registry.iter())
            .chain(held.channels.values())
            .chain(held.segments.iter())
            .copied()
            .collect()
    };
    let scope = state_helpers::login_scope_or_closed(state);
    for lease in leases {
        // Before each watch call: the session is ending, or the join
        // phase this runs in is out of budget (plan C4.L1b).
        if scope.is_closed() || rekindle_governance_runtime::phase_expired() {
            break;
        }
        let Some(record_key) = pool.key_of(lease) else {
            continue;
        };
        match pool.watch_all(lease).await {
            Ok(()) => {
                mark_watch_active(state, community_id, &record_key);
                tracing::debug!(community = %community_id, record_key, "watching community record");
            }
            Err(e) => tracing::warn!(
                community = %community_id,
                record_key,
                error = %e,
                "failed to watch community record"
            ),
        }
    }
    Ok(())
}
