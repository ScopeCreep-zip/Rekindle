//! Phase 23.D.4 — community-side sync helpers extracted from
//! `sync_service.rs` to keep that file under the 500-LoC cap.
//! Mesh-presence re-announce + governance pull + channel-record
//! discovery + per-channel watermark sync.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

use super::sync_service::request_channel_sync;

/// Sync communities by re-announcing our mesh presence.
pub(super) async fn sync_communities(state: &Arc<AppState>, pool: &DbPool) -> Result<(), String> {
    if state_helpers::safe_routing_context(state).is_none() {
        return Ok(()); // Not connected yet
    }

    let communities_with_governance = state_helpers::communities_with_governance_keys(state);
    for (community_id, governance_key) in &communities_with_governance {
        sync_community_governance(state, community_id, governance_key).await;
        sync_community_channels(state, pool, community_id).await;
        if let Err(e) = crate::services::community::rejoin_community(state, community_id).await {
            tracing::trace!(community = %community_id, error = %e, "community rejoin failed");
        }
    }

    tracing::debug!(
        communities = communities_with_governance.len(),
        "community sync complete"
    );
    Ok(())
}

pub(crate) async fn handle_community_record_change(
    state: &Arc<AppState>,
    pool: &DbPool,
    dht_key: &str,
) -> bool {
    enum ChangedRecord {
        Governance {
            community_id: String,
            governance_key: String,
        },
        Registry {
            community_id: String,
        },
        Channel {
            community_id: String,
            channel_id: String,
        },
    }

    let changed = {
        let communities = state.communities.read();
        communities.values().find_map(|community| {
            if community.governance_key.as_deref() == Some(dht_key) {
                return community.governance_key.as_ref().map(|governance_key| {
                    ChangedRecord::Governance {
                        community_id: community.id.clone(),
                        governance_key: governance_key.clone(),
                    }
                });
            }
            if community.member_registry_key.as_deref() == Some(dht_key) {
                return Some(ChangedRecord::Registry {
                    community_id: community.id.clone(),
                });
            }
            community
                .channel_log_keys
                .iter()
                .find_map(|(channel_id, record_key)| {
                    (record_key == dht_key).then(|| ChangedRecord::Channel {
                        community_id: community.id.clone(),
                        channel_id: channel_id.clone(),
                    })
                })
        })
    };

    match changed {
        Some(ChangedRecord::Governance {
            community_id,
            governance_key,
        }) => {
            sync_community_governance(state, &community_id, &governance_key).await;
            true
        }
        Some(ChangedRecord::Registry { community_id }) => {
            let _ =
                crate::services::community::presence_poll_tick_public(state, &community_id).await;
            true
        }
        Some(ChangedRecord::Channel {
            community_id,
            channel_id,
        }) => {
            request_channel_sync(state, pool, &community_id, &channel_id).await;
            true
        }
        None => false,
    }
}

fn report_fingerprint(seqs: &[veilid_core::ValueSeqNum]) -> u64 {
    let mut hasher = DefaultHasher::new();
    seqs.len().hash(&mut hasher);
    for seq in seqs {
        format!("{seq:?}").hash(&mut hasher);
    }
    hasher.finish()
}

async fn sync_community_governance(
    state: &Arc<AppState>,
    community_id: &str,
    governance_key: &str,
) {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return;
    };
    let Ok(record_key) = governance_key.parse::<veilid_core::RecordKey>() else {
        return;
    };
    let report = match rc
        .inspect_dht_record(
            record_key,
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            veilid_core::DHTReportScope::UpdateGet,
        )
        .await
    {
        Ok(report) => report,
        Err(e) => {
            tracing::trace!(community = %community_id, error = %e, "governance inspect failed");
            return;
        }
    };
    let fingerprint = report_fingerprint(report.network_seqs());
    let needs_rebuild = {
        let mut communities = state.communities.write();
        let Some(cs) = communities.get_mut(community_id) else {
            return;
        };
        let previous = cs
            .open_community_records
            .governance_report_fingerprint
            .replace(fingerprint);
        previous.is_some_and(|prev| prev != fingerprint) || cs.governance_state.is_none()
    };
    if needs_rebuild {
        tracing::info!(community = %community_id, "governance inspect changed — rebuilding merged state");
        crate::services::governance_adapter::rebuild_governance_from_dht(state).await;
        open_new_channel_records(state, community_id).await;
    }
}

async fn open_new_channel_records(state: &Arc<AppState>, community_id: &str) {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return;
    };
    let (channel_pairs, opened_keys) = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            return;
        };
        (
            cs.channel_log_keys
                .iter()
                .map(|(channel_id, record_key)| (channel_id.clone(), record_key.clone()))
                .collect::<Vec<_>>(),
            cs.open_community_records
                .channel_keys
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
        )
    };

    let mut newly_opened = Vec::new();
    for (_channel_id, record_key) in channel_pairs {
        if opened_keys.contains(&record_key) {
            continue;
        }
        let Ok(parsed_key) = record_key.parse::<veilid_core::RecordKey>() else {
            continue;
        };
        match rc.open_dht_record(parsed_key, None).await {
            Ok(_) => {
                newly_opened.push(record_key.clone());
                state_helpers::track_open_records(state, std::slice::from_ref(&record_key));
            }
            Err(e) => {
                tracing::trace!(
                    community = %community_id,
                    channel_record = %record_key,
                    error = %e,
                    "failed to open newly discovered channel record"
                );
            }
        }
    }

    if !newly_opened.is_empty() {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            for record_key in newly_opened {
                if !cs.open_community_records.channel_keys.contains(&record_key) {
                    cs.open_community_records.channel_keys.push(record_key);
                }
            }
        }
    }
}

async fn sync_community_channels(state: &Arc<AppState>, pool: &DbPool, community_id: &str) {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return;
    };
    let channel_pairs = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            return;
        };
        cs.channel_log_keys
            .iter()
            .map(|(channel_id, record_key)| (channel_id.clone(), record_key.clone()))
            .collect::<Vec<_>>()
    };

    for (channel_id, record_key) in channel_pairs {
        let Ok(parsed_key) = record_key.parse::<veilid_core::RecordKey>() else {
            continue;
        };
        let report = match rc
            .inspect_dht_record(
                parsed_key,
                Some(veilid_core::ValueSubkeyRangeSet::full()),
                veilid_core::DHTReportScope::UpdateGet,
            )
            .await
        {
            Ok(report) => report,
            Err(e) => {
                tracing::trace!(
                    community = %community_id,
                    channel = %channel_id,
                    error = %e,
                    "channel record inspect failed"
                );
                continue;
            }
        };
        let fingerprint = report_fingerprint(report.network_seqs());
        let should_request_sync = {
            let mut communities = state.communities.write();
            let Some(cs) = communities.get_mut(community_id) else {
                return;
            };
            let previous = cs
                .open_community_records
                .channel_report_fingerprints
                .insert(channel_id.clone(), fingerprint);
            previous.is_some_and(|prev| prev != fingerprint)
        };
        if should_request_sync {
            request_channel_sync(state, pool, community_id, &channel_id).await;
        }
    }
}
