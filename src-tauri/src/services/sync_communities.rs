//! Phase 23.D.4 — community-side sync helpers extracted from
//! `sync_service.rs` to keep that file under the 500-LoC cap.
//! Mesh-presence re-announce + governance pull + channel-record
//! discovery + per-channel watermark sync.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::sync_service::request_channel_sync;

/// Sync communities by re-announcing our mesh presence.
pub(super) async fn sync_communities(
    state: &Arc<AppState>,
    pool: &Db,
    stop: &tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    if state_helpers::safe_routing_context(state).is_none() {
        return Ok(()); // Not connected yet
    }

    let communities_with_governance = state_helpers::communities_with_governance_keys(state);
    for (community_id, governance_key) in &communities_with_governance {
        if stop.is_cancelled() {
            return Ok(());
        }
        sync_community_governance(state, community_id, governance_key).await;
        if stop.is_cancelled() {
            return Ok(());
        }
        sync_community_channels(state, pool, community_id, stop).await;
        if stop.is_cancelled() {
            return Ok(());
        }
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
    pool: &Db,
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
            if let Some(found) =
                community
                    .channel_log_keys
                    .iter()
                    .find_map(|(channel_id, record_key)| {
                        (record_key == dht_key).then(|| ChangedRecord::Channel {
                            community_id: community.id.clone(),
                            channel_id: channel_id.clone(),
                        })
                    })
            {
                return Some(found);
            }
            // Plate Gate (§15.4) — segment-N records route to the same
            // arms as their segment-0 counterparts: a segment-registry
            // change is a presence change, a segment-governance change
            // is a governance change, a channel-segment record change
            // is channel traffic. These are watched (tracked_watch_keys)
            // and previously fell through to the friend-presence
            // handler, which silently ignored them.
            let gov = community.governance_state.as_ref()?;
            if gov.segments.iter().any(|s| s.registry_key == dht_key) {
                return Some(ChangedRecord::Registry {
                    community_id: community.id.clone(),
                });
            }
            if gov.segments.iter().any(|s| s.governance_key == dht_key) {
                return Some(ChangedRecord::Governance {
                    community_id: community.id.clone(),
                    governance_key: dht_key.to_string(),
                });
            }
            gov.channel_segment_records
                .iter()
                .find_map(|((channel_id, _segment), csr)| {
                    (csr.record_key == dht_key).then(|| ChangedRecord::Channel {
                        community_id: community.id.clone(),
                        channel_id: hex::encode(channel_id.0),
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
    let Ok(pool) = state_helpers::record_pool(state) else {
        return;
    };
    let Ok(record_key) = governance_key.parse::<veilid_core::RecordKey>() else {
        return;
    };
    let report = match pool
        .inspect_once(&record_key, None, veilid_core::DHTReportScope::UpdateGet)
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

/// Borrow every channel record governance now names that the community does
/// not hold yet, and hand the leases to it (`leases::records_ready`: held,
/// recorded in the inventory, watched).
async fn open_new_channel_records(state: &Arc<AppState>, community_id: &str) {
    let Ok(pool) = state_helpers::record_pool(state) else {
        return;
    };
    let (channel_pairs, opened_keys, slot_writer) = {
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
            // Our slot writer: channel records share the registry's slot
            // seed, so a held channel write restored after a re-login
            // re-pushes as us (plan C7.13).
            cs.slot_keypair
                .as_deref()
                .and_then(|kp| kp.parse::<veilid_core::KeyPair>().ok()),
        )
    };

    let mut leases = rekindle_records::lease::CommunityLeases::default();
    for (channel_id, record_key) in channel_pairs {
        if opened_keys.contains(&record_key) {
            continue;
        }
        let (Ok(parsed_key), Some(channel)) = (
            record_key.parse::<veilid_core::RecordKey>(),
            hex::decode(&channel_id)
                .ok()
                .and_then(|b| <[u8; 16]>::try_from(b).ok())
                .map(rekindle_types::id::ChannelId),
        ) else {
            continue;
        };
        match pool.acquire(&parsed_key, slot_writer.clone()).await {
            Ok(lease) => {
                leases.channels.insert(channel, lease);
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
    if !leases.channels.is_empty() {
        crate::services::community::leases::records_ready(state, community_id, leases).await;
    }
}

async fn sync_community_channels(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    stop: &tokio_util::sync::CancellationToken,
) {
    let Ok(record_pool) = state_helpers::record_pool(state) else {
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
        if stop.is_cancelled() {
            return;
        }
        let Ok(parsed_key) = record_key.parse::<veilid_core::RecordKey>() else {
            continue;
        };
        let report = match record_pool
            .inspect_once(&parsed_key, None, veilid_core::DHTReportScope::UpdateGet)
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
