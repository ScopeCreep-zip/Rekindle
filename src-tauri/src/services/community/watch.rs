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

/// The writer to open `record_key` with if a watch-time heal needs to
/// open it.
///
/// Veilid re-opens replace the record's stored writer in place, so a
/// blanket read-only open here would silently strip write permission
/// from the member registry — the exact clobber
/// `open_and_track_one_community` documents. The registry therefore
/// opens with the same writer login uses; every other class opens
/// read-only, matching the convention that writes pass the slot
/// keypair inline at `set_dht_value` time.
fn heal_open_writer(
    state: &Arc<AppState>,
    community_id: &str,
    record_key: &str,
) -> Option<veilid_core::KeyPair> {
    let communities = state.communities.read();
    let community = communities.get(community_id)?;
    if community.member_registry_key.as_deref() != Some(record_key) {
        return None;
    }
    community
        .registry_owner_keypair
        .as_deref()
        .or(community.slot_keypair.as_deref())
        .and_then(|s| s.parse::<veilid_core::KeyPair>().ok())
}

/// Re-open a community record that Veilid reports as not open.
///
/// Both delivery legs — watch establishment and the inspect/poll loop —
/// call Veilid primitives that require the record open *this session*,
/// and both hit `record not open` when a login open failed on a cold
/// network or Veilid later dropped the handle (route refresh, eviction).
/// Re-opening is idempotent and returns from the local store without a
/// network round-trip when the record is already local (vendored 0.5.7
/// `open_dht_record` docs), and it preserves any active watch — so this
/// is safe to call on every failure. Opens the registry with its writer
/// (a read-only re-open would strip presence/slot write capability);
/// everything else read-only.
///
/// Returns `true` if the record is open afterward.
pub(crate) async fn reopen_record(
    rc: &veilid_core::RoutingContext,
    state: &Arc<AppState>,
    community_id: &str,
    parsed_key: &veilid_core::RecordKey,
    record_key: &str,
) -> bool {
    let writer = heal_open_writer(state, community_id, record_key);
    match rc.open_dht_record(parsed_key.clone(), writer).await {
        Ok(_) => {
            tracing::info!(
                community = %community_id,
                record_key,
                "re-opened record Veilid reported as not open"
            );
            true
        }
        Err(e) => {
            // TryAgain while offline / KeyNotFound while still
            // propagating — the caller's next tick re-enters here.
            tracing::debug!(
                community = %community_id,
                record_key,
                error = %e,
                "record re-open failed — will retry next tick"
            );
            false
        }
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
    let mut result = rc
        .watch_dht_values(
            parsed_key.clone(),
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            None,
            None,
        )
        .await;

    // Veilid watches require the record open THIS session ("Can only be
    // used on opened records" — an InvalidArgument otherwise). The open
    // is best-effort at login, so a record whose open failed on a cold
    // network — or was never attempted, e.g. a channel discovered from
    // a governance rebuild — used to fail its watch on every retry tick
    // forever: observed live as 500+ 'record not open' errors per
    // session, with channel messages arriving only via the poll
    // backstop. Heal the open here, where the retry loop already
    // targets exactly the records that are not being watched.
    if matches!(
        result,
        Err(veilid_core::VeilidAPIError::InvalidArgument { .. })
    ) && reopen_record(rc, state, community_id, &parsed_key, record_key).await
    {
        // reopen_record logs a failed re-open; the next retry tick
        // re-enters this path, so a still-propagating record heals then.
        result = rc
            .watch_dht_values(
                parsed_key,
                Some(veilid_core::ValueSubkeyRangeSet::full()),
                None,
                None,
            )
            .await;
    }

    match result {
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
